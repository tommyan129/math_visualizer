// Highlights against the resolved state written by the Rust resolver
// (`cargo test -p dsl --test web_fixtures`): regions, fills, rasters, LaTeX coloring.

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/state.json";
import type { State } from "../src/generated/State";
import type { Highlight } from "../src/generated/Highlight";
import { compile, compileRegion, type Scalar } from "../src/worker/codegen";
import { computeFill, rasterRegion } from "../src/worker/fill";
import { makeRuntime } from "../src/worker/numeric";
import { maskPixels, resolveHighlights } from "../src/canvas/highlight";
import { definitionTex, exprTex } from "../src/math/latex";

const state = fixture as unknown as State;
const fns = Object.values(state.functions).map((f) => f!);
const constKeys = Object.keys(state.constants);
const env = { constIndex: new Map(constKeys.map((k, i) => [k, i])), fnIndex: new Map(fns.map((f, i) => [f.key, i])) };
const R = makeRuntime();
const F: Scalar[] = [];
fns.forEach((f, i) => (F[i] = compile({ key: f.key, version: f.version, body: f.body, consts: f.consts, calls: f.calls }, env, R, F, false).scalar));
const C = Float64Array.from(constKeys.map((k) => state.constants[k]!.value));
const hl = (k: string) => state.highlights[k] as Highlight;
const region = (k: string) => compileRegion(hl(k).range!, env, R, F);
const fn = (k: string) => F[env.fnIndex.get(k)!]!;
const range = { x0: -5, x1: 5, y0: -5, y1: 5, sx: 40, sy: 40 };

describe("regions", () => {
  it("between(f, g) fills on both sides of a crossing", () => {
    // H2 = highlight(; between(lin, quad) and 0 <= x <= 2): lin = x, quad = x², crossing at x = 1.
    const r = region("H2");
    expect(r(0.5, 0.4, C)).toBe(true); // x² < y < x
    expect(r(1.5, 2, C)).toBe(true); // x < y < x²
    expect(r(0.5, 0.6, C)).toBe(false);
    expect(r(1.5, 1.4, C)).toBe(false);
    expect(r(2.5, 4, C)).toBe(false); // outside 0 <= x <= 2
  });

  it("or / not / constants and calls", () => {
    expect(region("H5")(-2, 0, C)).toBe(true);
    expect(region("H5")(0, 0, C)).toBe(false);
    // H6: y < par(x) and not (x > a), a = 1, par = a*x²
    expect(region("H6")(0.5, 0.1, C)).toBe(true);
    expect(region("H6")(2, 0.1, C)).toBe(false);
  });

  it("raster mask, row 0 at the top", () => {
    const m = rasterRegion(region("H2"), { x0: 0, x1: 2, y0: 0, y1: 4 }, 20, 40, C);
    const at = (x: number, y: number) => m[Math.floor((4 - y) / 0.1) * 20 + Math.floor(x / 0.1)];
    expect(at(0.55, 0.4)).toBe(1);
    expect(at(1.55, 2.0)).toBe(1);
    expect(at(0.55, 3.5)).toBe(0);
    const px = maskPixels(m, "#5fd38a", 0.5);
    expect(px.length).toBe(m.length * 4);
  });
});

describe("function fills", () => {
  it("fills to the axis only inside the range, with exact edges", () => {
    // H1 = highlight(quad; R1) with R1 = 0 <= x <= 2.
    const g = computeFill(fn("quad"), region("H1"), "Axis", range, 1000, C);
    expect(g.rings.length).toBe(2); // one ring
    const xs = g.edge.filter((_, i) => i % 2 === 0);
    expect(Math.min(...xs)).toBeCloseTo(0, 6);
    expect(Math.max(...xs)).toBeCloseTo(2, 6);
    // Ring closes along the axis.
    const n = g.xy.length;
    expect(g.xy[n - 1]).toBe(0);
    expect(g.xy[n - 3]).toBe(0);
  });

  it("an `or` range gives separate pieces; above / below use the computed area", () => {
    const g = computeFill(fn("lin"), region("H5"), "Axis", range, 1000, C);
    expect(g.rings.length - 1).toBe(2);
    const a = computeFill(fn("lin"), null, "Above", range, 100, C);
    expect(a.xy[a.xy.length - 1]).toBe(15); // y1 + one view height
    const b = computeFill(fn("lin"), null, "Below", range, 100, C);
    expect(b.xy[b.xy.length - 1]).toBe(-15);
  });

  it("undefined points break the fill", () => {
    const sq = fns.find((f) => f.key === "sq")!;
    const g = computeFill(fn(sq.key), null, "Axis", range, 1000, C);
    const xs = g.edge.filter((_, i) => i % 2 === 0);
    expect(Math.min(...xs)).toBeCloseTo(0, 6);
  });
});

describe("overlap and expression coloring", () => {
  it("subexpression nodes are colored last-wins; hidden ones are skipped", () => {
    const { fills, nodeColors } = resolveHighlights(state, new Set());
    expect(fills.map((h) => h.key)).toEqual(["H1", "H2", "H5", "H6"]);
    const poly = nodeColors.get("poly")!;
    // H3 colors b (orange); H4 colors the whole term -b*x (red).
    const h3 = hl("H3").target, h4 = hl("H4").target;
    if (h3.type !== "Sub" || h4.type !== "Sub") throw new Error("sub targets");
    expect(poly.get(h3.nodes[0]!)).toBe("#ffa057");
    expect(poly.get(h4.nodes[0]!)).toBe("#ff6b6b");
    const hidden = resolveHighlights(state, new Set(["H4"])).nodeColors.get("poly")!;
    expect(hidden.has(h4.nodes[0]!)).toBe(false);
  });

  it("LaTeX wraps the addressed nodes; a term keeps its sign inside the color", () => {
    const f = state.functions["poly"]!;
    const h3 = hl("H3").target;
    if (h3.type !== "Sub") throw new Error();
    const tex = definitionTex(f.name, f.inputs, f.body, { colors: new Map(), nodes: new Map([[h3.nodes[0]!, "#ffa057"]]) });
    expect(tex).toContain("\\textcolor[RGB]{255,160,87}{b}");
    const h4 = hl("H4").target;
    if (h4.type !== "Sub") throw new Error();
    const t4 = exprTex(f.body, { colors: new Map(), nodes: new Map([[h4.nodes[0]!, "#ff6b6b"]]) });
    expect(t4).toContain("\\mathbin{\\textcolor[RGB]{255,107,107}{-}}");
  });

  it("ranges render as inequalities", () => {
    expect(exprTex(hl("H5").source.range!, { colors: new Map() })).toBe("x < -1 \\;\\text{or}\\; x > 1");
  });
});
