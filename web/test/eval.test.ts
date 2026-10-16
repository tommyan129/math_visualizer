// Numeric semantics, codegen and sampling against ASTs produced by the Rust resolver
// (fixtures written by `cargo test -p dsl --test web_fixtures`).

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/state.json";
import type { State } from "../src/generated/State";
import { compile, compileOrder, type Compiled, type Scalar, specOf } from "../src/worker/codegen";
import { clipLine, spanShape } from "../src/worker/linalg";
import { makeRuntime, TERM_CAP } from "../src/worker/numeric";
import { sampleImplicit } from "../src/worker/implicit";
import { sampleParametric } from "../src/worker/paramsample";
import { isDiscontinuity, sampleCurve, sampleDiscrete, type SampleRange } from "../src/worker/sample";

const state = fixture as unknown as State;
const fns = Object.values(state.functions).map((f) => f!);
const constKeys = Object.keys(state.constants);
const constIndex = new Map(constKeys.map((k, i) => [k, i]));
const fnIndex = new Map(fns.map((f, i) => [f.key, i]));
const R = makeRuntime();
const F: Scalar[] = [];
const compiled = new Map<string, Compiled>();
fns.forEach((f, i) => {
  const c = compile(specOf(f), { constIndex, fnIndex }, R, F, false);
  compiled.set(f.key, c);
  F[i] = c.table;
});

function C(values: Record<string, number> = {}): Float64Array {
  return Float64Array.from(constKeys.map((k) => values[k] ?? state.constants[k]!.value));
}

function f(key: string, x: number, consts?: Record<string, number>): number {
  return compiled.get(key)!.scalar(x, C(consts));
}

describe("numeric semantics", () => {
  it("rational powers of negative bases", () => {
    expect(f("cbrt", -8)).toBeCloseTo(-2);
    expect(f("cbrt2", -8)).toBeCloseTo(4);
    expect(f("half", -4)).toBeNaN();
    // A decimal exponent is not a fraction: plain real pow.
    expect(f("dec", -4)).toBeNaN();
  });

  it("unary minus binds looser than power", () => {
    expect(f("negsq", 0)).toBe(-4);
  });

  it("rounding, modulo, factorial, gamma", () => {
    expect(f("rnd", 2.5)).toBe(3);
    expect(f("rnd", -2.5)).toBe(-3);
    expect(f("md", -1)).toBe(2);
    expect(f("md", 7)).toBe(1);
    expect(f("fct", 5)).toBe(120);
    expect(f("fct", 0)).toBe(1);
    expect(f("fct", 2.5)).toBeNaN();
    expect(f("fct", -1)).toBeNaN();
    expect(f("gm", 5)).toBeCloseTo(24, 8);
    expect(f("gm", 0.5)).toBeCloseTo(Math.sqrt(Math.PI), 8);
    expect(f("gm", 0)).toBeNaN();
  });

  it("log and reserved values", () => {
    // log(8, 2) + ln(e) + deg(180) - pi = 3 + 1 + pi - pi
    expect(f("lg", 8)).toBeCloseTo(4);
  });

  it("0^0 = 1", () => {
    expect(R.pow(0, 0)).toBe(1);
  });

  it("sum with an integer constant bound", () => {
    expect(f("taylor", 1, { n: 20 })).toBeCloseTo(Math.E, 10);
    expect(f("taylor", 2, { n: 0 })).toBe(1);
  });

  it("sum with x in its bound is defined only at integers", () => {
    expect(f("harm", 3)).toBeCloseTo(1 + 1 / 2 + 1 / 3);
    expect(f("harm", 2.5)).toBeNaN();
    expect(f("harm", 0)).toBe(0);
    expect(compiled.get("harm")!.discrete).toBe(true);
  });

  it("sum over too many terms is NaN and flagged", () => {
    R.capped = false;
    expect(f("big", 0)).toBeNaN();
    expect(R.capped).toBe(true);
    expect(TERM_CAP).toBeLessThan(1_000_000);
  });

  it("user function calls with constants", () => {
    expect(f("callpar", 2, { a: 3 })).toBeCloseTo(3 * 9 - 3 * 4);
  });
});

const view: SampleRange = { x0: -20, x1: 20, y0: -12.5, y1: 12.5, sx: 40, sy: 40 };

function pieces(key: string, range = view) {
  const c = compiled.get(key)!;
  return sampleCurve(c.batch, c.scalar, C(), range, "fine");
}

describe("sampling", () => {
  it("1/x breaks at 0 instead of drawing a vertical line", () => {
    const p = pieces("recip");
    expect(p.offsets.length - 1).toBe(2);
    // No segment crosses x = 0.
    for (let s = 0; s + 1 < p.offsets.length; s++) {
      const a = p.xs[p.offsets[s]!]!, b = p.xs[p.offsets[s + 1]! - 1]!;
      expect(Math.sign(a)).toBe(Math.sign(b));
    }
  });

  it("tan breaks at every asymptote in view", () => {
    const p = pieces("tn");
    // Asymptotes at pi/2 + k*pi inside [-20, 20]: k = -6..5, 12 of them → 13 pieces.
    expect(p.offsets.length - 1).toBe(13);
  });

  it("steep continuous functions stay connected", () => {
    expect(pieces("steep").offsets.length - 1).toBe(1);
    expect(pieces("cbrt").offsets.length - 1).toBe(1);
  });

  it("sqrt approaches its domain edge", () => {
    const p = pieces("sq");
    expect(p.offsets.length - 1).toBe(1);
    expect(p.xs[0]!).toBeGreaterThanOrEqual(0);
    expect(p.xs[0]!).toBeLessThan(0.01);
  });

  it("floor breaks at each integer", () => {
    const p = pieces("fl", { ...view, x0: -3, x1: 3 });
    expect(p.offsets.length - 1).toBe(6);
  });

  it("discrete functions sample integers only", () => {
    const c = compiled.get("harm")!;
    const d = sampleDiscrete(c.batch, C(), { ...view, x0: -2.5, x1: 4.2 });
    // Integers only; below 1 the sum is empty, so 0.
    expect(Array.from(d.xs)).toEqual([-2, -1, 0, 1, 2, 3, 4]);
    expect(d.ys[0]).toBe(0);
  });

  it("isDiscontinuity tells a jump from a steep slope", () => {
    const step: Scalar = (x) => (x < 0 ? 0 : 5);
    const steep: Scalar = (x) => 1000 * x;
    expect(isDiscontinuity(step, new Float64Array(), -0.01, 0, 0.01, 5)).toBe(true);
    expect(isDiscontinuity(steep, new Float64Array(), -0.01, -10, 0.01, 10)).toBe(false);
  });
});

// Regressions from docs/improvements.md.
describe("sampling regressions", () => {
  const fn = (f: (x: number) => number) => ({
    scalar: ((x: number) => f(x)) as Scalar,
    batch: (xs: Float64Array, out: Float64Array) => {
      for (let i = 0; i < xs.length; i++) out[i] = f(xs[i]!);
    },
  });
  /** Computed range as the plot builds it: visible box plus one screen each side. */
  const range = (x0: number, x1: number, y0: number, y1: number, widthPx: number): SampleRange => {
    const w = x1 - x0, h = y1 - y0, s = widthPx / w;
    return { x0: x0 - w, x1: x1 + w, y0: y0 - h, y1: y1 + h, sx: s, sy: s };
  };

  it("a zoomed-out parabola does not refine off-screen parts (W-budget)", () => {
    const { batch, scalar } = fn((x) => x * x + x + 1);
    const p = sampleCurve(batch, scalar, new Float64Array(), range(-315, 353, -12, 354, 880), "fine");
    expect(p.truncated).toBe(false);
    expect(p.xs.length).toBeLessThan(5000);
    expect(p.offsets.length - 1).toBe(1);
  });

  it("huge coefficients neither truncate nor break a continuous curve", () => {
    for (const a of [1e6, 1e9, 1e12, 1e15]) {
      const { batch, scalar } = fn((x) => a * x * x + x + 1);
      const p = sampleCurve(batch, scalar, new Float64Array(), range(-10, 10, -6, 6, 880), "fine");
      expect(p.truncated).toBe(false);
      expect(p.offsets.length - 1).toBe(1);
    }
  });

  it("a scaled step is still a discontinuity", () => {
    const { batch, scalar } = fn((x) => (x < 0.3 ? 0 : 1e12));
    const p = sampleCurve(batch, scalar, new Float64Array(), range(-10, 10, -6, 6, 880), "fine");
    // Broken at the step: the visible part and the (clipped) upper part.
    expect(p.offsets.length - 1).toBe(2);
    const end = p.xs[p.offsets[1]! - 1]!, start = p.xs[p.offsets[1]!]!;
    expect(end).toBeLessThan(0.3);
    expect(start).toBeGreaterThanOrEqual(0.3);
  });

  it("small jumps (a few px when zoomed out) still break the line", () => {
    // Midpoint tangent lines per unit cell of f(x) = x^4/40 - x/2 + 2: jumps of 0.1–0.3 at integers.
    const f = (x: number) => x ** 4 / 40 - x / 2 + 2;
    const tangent = (x: number) => {
      const m = Math.floor(x) + 0.5;
      return f(m) + (m ** 3 / 10 - 0.5) * (x - m);
    };
    const { batch, scalar } = fn(tangent);
    // Zoomed out: 60 px per unit, so every jump is under jumpPx.
    const px = 60;
    const p = sampleCurve(batch, scalar, new Float64Array(), range(-3, 4, -1, 8, 7 * px), "fine");
    const breaks: number[] = [];
    for (let k = 1; k + 1 < p.offsets.length; k++) breaks.push(p.xs[p.offsets[k]! - 1]!);
    let checked = 0;
    for (let c = -3; c <= 4; c++) {
      const jumpPx = Math.abs(tangent(c) - tangent(c - 1e-9)) * px;
      expect(jumpPx).toBeLessThan(30);
      // Near x = 0 the jump vanishes (f'' = 0); only visible ones must break.
      if (jumpPx < 4) continue;
      checked++;
      expect(breaks.some((b) => b < c && c - b < 0.05)).toBe(true);
    }
    expect(checked).toBeGreaterThanOrEqual(5);
  });

  it("a steep continuous curve is not broken by the small-jump check", () => {
    const { batch, scalar } = fn((x) => Math.sin(x) * 3 + x * x * x);
    const p = sampleCurve(batch, scalar, new Float64Array(), range(-3, 4, -1, 8, 420), "fine");
    expect(p.offsets.length - 1).toBe(1);
  });
});

describe("vectors and matrices", () => {
  it("a matrix function called inside a curve", () => {
    // rotx(x; b) = (R(b)·[x, 1])_2 = sin(b)·x + cos(b)
    for (const [x, b] of [[0, 1], [2, 0.5], [-3, 2]] as const) {
      expect(f("rotx", x, { b })).toBeCloseTo(Math.sin(b) * x + Math.cos(b), 12);
    }
  });

  it("det, tr, inv, pow, transpose and elements", () => {
    // M2 = [[1, 2], [3, 4]]: det -2, tr(inv) -2.5, (M2²)₁₂ 10, (Mᵀ)₁₂ 3, (M⁻¹)₁₁ -2
    expect(f("linalg", 0)).toBeCloseTo(8.5, 12);
    expect(f("linalg", 1)).toBeCloseTo(6.5, 12);
  });

  it("outer product and element numbers outside the vector are NaN", () => {
    expect(f("outer", 2, { k: 1 })).toBeCloseTo(7, 12);
    expect(f("outer", 2, { k: 3 })).toBeCloseTo(9, 12);
    expect(f("outer", 2, { k: 4 })).toBeNaN();
    expect(f("outer", 2, { k: 1.5 })).toBeNaN();
  });

  it("singular inverse and non-integer powers are NaN", () => {
    expect(R.inv([1, 2, 2, 4], 2).every(Number.isNaN)).toBe(true);
    expect(R.mpow([1, 2, 3, 4], 2, 0.5).every(Number.isNaN)).toBe(true);
    expect(R.mpow([2, 0, 0, 3], 2, 0)).toEqual([1, 0, 0, 1]);
    expect(R.det([0, 1, 1, 0], 2)).toBe(-1);
    expect(R.det([2, 0, 0, 0, 3, 0, 0, 0, 4], 3)).toBe(24);
  });
});

describe("span shape", () => {
  const vecs = (key: string, consts?: Record<string, number>) => compiled.get(key)!.vecs.map((v) => v(C(consts)));

  it("plane, line, point from current values", () => {
    expect(spanShape(vecs("Sp", { a: 1 })).dim).toBe(2);
    expect(spanShape(vecs("Sp", { a: 0 })).dim).toBe(2);
    const line = spanShape(vecs("Sl"));
    expect(line.dim).toBe(1);
    expect(line.dir![0]).toBeCloseTo(1 / Math.sqrt(5), 12);
    expect(spanShape(vecs("Sz")).dim).toBe(0);
  });

  it("near-parallel tolerance is relative", () => {
    expect(spanShape([[1, 2], [2, 4 + 1e-12]]).dim).toBe(1);
    expect(spanShape([[1e-6, 2e-6], [2e-6, 4.001e-6]]).dim).toBe(2);
    expect(spanShape([[1e6, 0], [1e-9, 0]]).dim).toBe(1);
    expect(spanShape([[0, 0], [1e-30, 1e-30]]).dim).toBe(1);
    expect(spanShape([[NaN, 1]]).dim).toBeNull();
  });

  it("line clipped to the box", () => {
    expect(clipLine([1, 0], { x0: -2, x1: 3, y0: -1, y1: 1 })!.map((v) => v + 0)).toEqual([-2, 0, 3, 0]);
    expect(clipLine([0, 1], { x0: 1, x1: 3, y0: -1, y1: 1 })).toBeNull();
  });

  it("arrow vector and start point", () => {
    const [v, p] = vecs("Ar", { a: 3 });
    expect(v).toEqual([1, 3]);
    expect(p).toEqual([1, 1]);
  });
});

describe("split", () => {
  it("picks the line whose condition holds", () => {
    expect(f("absx", -3)).toBe(3);
    expect(f("absx", 2)).toBe(2);
    expect(f("absx", 0)).toBe(0);
    expect(f("stp", 0, { a: 1, b: 2 })).toBe(0);
    expect(f("stp", 1.5, { a: 1, b: 2 })).toBe(1);
    expect(f("stp", 2, { a: 1, b: 2 })).toBe(1);
    expect(f("stp", 3, { a: 1, b: 2 })).toBe(2);
  });

  it("the subject name stands for the expression", () => {
    expect(f("sq2", 0.5)).toBeCloseTo(0.25);
    expect(f("sq2", 2)).toBeCloseTo(0.25);
  });

  it("a piece no line claims is undefined", () => {
    expect(f("gap", 0)).toBeNaN();
    expect(f("gap", -1e-9)).toBe(-1);
  });

  it("overlap at the current values is undefined and flagged", () => {
    R.overlap = false;
    expect(f("stp", 1.5, { a: 2, b: 1 })).toBeNaN();
    expect(R.overlap).toBe(true);
    R.overlap = false;
    expect(f("stp", 0, { a: 2, b: 1 })).toBe(0);
    expect(R.overlap).toBe(false);
  });

  it("boundary order constraints from the server", () => {
    const c = state.functions["stp"]!.constraints;
    expect(c.map((x) => x.src)).toEqual(["a < b"]);
    const holds = compileOrder(c[0]!.lo, c[0]!.hi, constIndex, R);
    expect(holds(C({ a: 1, b: 2 }))).toBe(true);
    expect(holds(C({ a: 2, b: 2 }))).toBe(false);
  });

  it("splits sample with a break at a jump", () => {
    const range: SampleRange = { x0: -3, x1: 3, y0: -3, y1: 3, sx: 100, sy: 100 };
    const c = compiled.get("stp")!;
    const r = sampleCurve(c.batch, c.scalar, C({ a: -1, b: 1 }), range, "fine");
    expect(r.offsets.length - 1).toBeGreaterThanOrEqual(3);
  });
});

describe("parametric, polar and implicit curves", () => {
  const range: SampleRange = { x0: -6, x1: 6, y0: -4, y1: 4, sx: 100, sy: 100 };
  const near = (a: number[], b: number[]) => Math.hypot(a[0]! - b[0]!, a[1]! - b[1]!) < 1e-9;

  it("parametric and polar points, ranges", () => {
    const pc = compiled.get("pc")!, hc = compiled.get("hc")!, pv = compiled.get("pv")!;
    const p = pc.param!(Math.PI / 2, C());
    expect(p[0]).toBeCloseTo(0, 12);
    expect(p[1]).toBeCloseTo(1, 12);
    expect(hc.param!(0, C())).toEqual([2, 0]);
    expect(hc.scalar(Math.PI, C())).toBeCloseTo(0, 12);
    expect(hc.trange!(C())).toEqual([0, 2 * Math.PI]);
    expect(pc.trange!(C())[1]).toBeCloseTo(2 * Math.PI, 12);
    // A vector-valued body: R(t)·[a, 0].
    const q = pv.param!(Math.PI / 2, C({ a: 3 }));
    expect(q[0]).toBeCloseTo(0, 12);
    expect(q[1]).toBeCloseTo(3, 12);
  });

  it("a parametric circle is one closed piece", () => {
    const pc = compiled.get("pc")!;
    const r = sampleParametric(pc.param!, C(), 0, 2 * Math.PI, range, "fine");
    expect(r.offsets).toEqual([0, r.xs.length]);
    expect(near([r.xs[0]!, r.ys[0]!], [r.xs[r.xs.length - 1]!, r.ys[r.ys.length - 1]!])).toBe(true);
    for (let i = 0; i < r.xs.length; i++) expect(Math.hypot(r.xs[i]!, r.ys[i]!)).toBeCloseTo(1, 9);
  });

  it("x^2 + y^2 = 1 is one closed polyline on the circle", () => {
    const ci = compiled.get("Ci")!;
    expect(ci.implicit!(0.6, 0.8, C())).toBeCloseTo(0, 12);
    const r = sampleImplicit(ci.implicit!, C(), range, "fine");
    expect(r.offsets).toEqual([0, r.xs.length]);
    expect(r.xs.length).toBeGreaterThan(100);
    const n = r.xs.length;
    expect(Math.hypot(r.xs[0]! - r.xs[n - 1]!, r.ys[0]! - r.ys[n - 1]!)).toBeLessThan(1e-9);
    for (let i = 0; i < n; i++) expect(Math.abs(Math.hypot(r.xs[i]!, r.ys[i]!) - 1)).toBeLessThan(1e-6);
  });

  it("xy = 1 is not joined across its pole", () => {
    const r = sampleImplicit(compiled.get("Hy")!.implicit!, C(), range, "fine");
    expect(r.offsets.length - 1).toBeGreaterThanOrEqual(2);
    for (let p = 0; p + 1 < r.offsets.length; p++) {
      const s = r.offsets[p]!, e = r.offsets[p + 1]!;
      const sign = Math.sign(r.xs[s]!);
      for (let i = s; i < e; i++) expect(Math.sign(r.xs[i]!)).toBe(sign);
    }
  });
});

describe("family", () => {
  const range: SampleRange = { x0: -6, x1: 6, y0: -4, y1: 4, sx: 100, sy: 100 };
  it("one copy per k, the index in the body and the range", () => {
    const dv = compiled.get("Dv")!;
    expect(dv.family!.krange(C({ n: 3 }))).toEqual([0, 3]);
    const m = dv.family!.at(2);
    expect(m.param!(1.5, C())).toEqual([2, 1.5]);
    expect(m.trange!(C())).toEqual([0, 4]);
  });

  it("concentric circles are three closed polylines", () => {
    const oc = compiled.get("Oc")!;
    for (const k of [1, 2, 3]) {
      const r = sampleImplicit(oc.family!.at(k).implicit!, C(), { ...range, x0: -8, x1: 8, y0: -8, y1: 8 }, "fine");
      expect(r.offsets.length).toBe(2);
      expect(Math.abs(Math.hypot(r.xs[0]!, r.ys[0]!) - k)).toBeLessThan(1e-6);
    }
  });
});
