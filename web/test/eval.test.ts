// Numeric semantics, codegen and sampling against ASTs produced by the Rust resolver
// (fixtures written by `cargo test -p dsl --test web_fixtures`).

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/state.json";
import type { State } from "../src/generated/State";
import { compile, type Compiled, type Scalar } from "../src/worker/codegen";
import { makeRuntime, TERM_CAP } from "../src/worker/numeric";
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
  const c = compile({ key: f.key, version: f.version, body: f.body, consts: f.consts, calls: f.calls }, { constIndex, fnIndex }, R, F, false);
  compiled.set(f.key, c);
  F[i] = c.scalar;
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
    const p = pieces("inv");
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
