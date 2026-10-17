// Stage 3 against the Rust-resolved fixture: scene lookups, axis mapping in regions,
// view fitting, note rendering input.

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/state.json";
import type { State } from "../src/generated/State";
import { activeClip, concealedAt, sceneAxes, sortedScenes, visibleAt } from "../src/canvas/scene";
import { compile, compileConst, compileRegion, type Scalar, specOf } from "../src/worker/codegen";
import { makeRuntime } from "../src/worker/numeric";
import { rasterRegion } from "../src/worker/fill";

const state = fixture as unknown as State;
const R = makeRuntime();
const fns = Object.values(state.functions).map((f) => f!);
const constKeys = Object.keys(state.constants).sort();
const constIndex = new Map(constKeys.map((k, i) => [k, i]));
const fnIndex = new Map(fns.map((f, i) => [f.key, i]));
const F: Scalar[] = [];
fns.forEach((f, i) => (F[i] = compile(specOf(f), { constIndex, fnIndex }, R, F, false).table));
const C = Float64Array.from(constKeys.map((k) => state.constants[k]!.value));
const sc = state.scenes["1"]!;

describe("scene model", () => {
  it("default scenes per coordinate system, Claude scenes after them", () => {
    expect(sortedScenes(Object.values(state.scenes).map((s) => s!)).map((s) => s.key)).toEqual(["2D(x, y)", "2D(t, y)", "polar(r, \\theta)", "3D(x, y, z)", "1"]);
    expect(state.scenes["polar(r, \\theta)"]!.polar).toBe(true);
    expect(sceneAxes(sc)[0]).toEqual(["t", "y"]);
    // Length-2 spans and arrows fit every 2D system.
    expect(sc.members).toEqual(["wave", "Sp", "Sl", "Sz", "Ar", "H7", "Af"]);
  });

  it("visibility: user override, else the step's state", () => {
    const d = state.scenes["2D(x, y)"]!;
    expect(visibleAt(d, 0, "quad")).toBe(true);
    expect(visibleAt(d, 0, "lin")).toBe(false); // the user turned it off
    expect(visibleAt(d, 0, "H1")).toBe(false); // highlights start off
    expect(visibleAt(d, 0, "wave")).toBe(false); // not a member
    expect(visibleAt(sc, 0, "wave")).toBe(true);
    expect([visibleAt(sc, 0, "H7"), visibleAt(sc, 1, "H7")]).toEqual([false, true]);
    expect([...concealedAt(sc, 0)]).toEqual(["H7"]);
    expect(concealedAt(sc, 1).size).toBe(0);
  });

  it("step caches: values, animate, view and notes", () => {
    expect(sc.steps[1]!.values).toEqual({ a: 2 });
    expect(sc.steps[2]!.animate).toEqual([{ constant: "a", from: 1, to: 3, duration: 0.5, looped: true }]);
    expect(sc.steps[2]!.changed_view).toBe(true);
    const segs = sc.steps[1]!.note[0]!.segs.map((g) => g.type);
    expect(segs).toEqual(["Def", "Text", "Tex", "Text"]);
    expect(sc.steps[1]!.note[0]!.segs[3]).toEqual({ type: "Text", text: " {넓이}" });
    expect(sc.steps[2]!.note.length).toBe(2);
  });
});

describe("regions on other axes", () => {
  it("t maps to the horizontal parameter", () => {
    const h7 = state.highlights["H7"]!;
    const region = compileRegion(h7.range!, { constIndex, fnIndex, axes: ["t", "y"] }, R, F);
    expect(region(0.5, -0.1, C)).toBe(true);
    expect(region(1.5, -0.1, C)).toBe(false);
    expect(region(0.5, 0.9, C)).toBe(false); // a = 1: sin(0.5) ≈ 0.48
  });

  it("the clip is a region over the scene axes", () => {
    const clip = activeClip(sc)!;
    const region = compileRegion(clip.expr, { constIndex, fnIndex, axes: ["t", "y"] }, R, F);
    const mask = rasterRegion(region, { x0: 0, x1: 1, y0: -1, y1: 1 }, 2, 4, C);
    // Rows from the top: y = 0.75, 0.25, -0.25, -0.75.
    expect([...mask]).toEqual([1, 1, 1, 1, 1, 1, 0, 0]);
  });
});

describe("view", () => {
  it("bounds are constant-free expressions", () => {
    const v = sc.view!;
    expect(compileConst(v.h!.lo, R)()).toBeCloseTo(-2 * Math.PI);
    expect(compileConst(v.v!.hi, R)()).toBe(2);
  });
});
