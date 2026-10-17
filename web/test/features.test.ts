// Feature points (stage 5b) against the state the Rust resolver produces, through the
// same path as the worker: definitions plus generated derivatives, then the feature set.
// Fixture written by `cargo test -p dsl --test web_fixtures`.

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/features.json";
import type { FeatureClass } from "../src/generated/FeatureClass";
import type { State } from "../src/generated/State";
import { compile, specsOf, type Compiled, type Scalar } from "../src/worker/codegen";
import { FeatureSet, roots, signChanges } from "../src/worker/features";
import { makeRuntime } from "../src/worker/numeric";
import type { SampleRange } from "../src/worker/sample";

const state = fixture as unknown as State;
const fns = Object.values(state.functions).map((f) => f!).sort((a, b) => a.order - b.order);
const specs = specsOf(state, fns);
const constKeys = Object.keys(state.constants).sort();
const constIndex = new Map(constKeys.map((k, i) => [k, i]));
const fnIndex = new Map(specs.map((f, i) => [f.key, i]));
const R = makeRuntime();
const F: Scalar[] = [];
const compiled = new Map<string, Compiled>();
const errors = new Map<string, string>();
specs.forEach((s, i) => {
  const c = compile(s, { constIndex, fnIndex }, R, F, false);
  compiled.set(s.key, c);
  F[i] = c.table;
});
const set = new FeatureSet();
set.set(specs, { constIndex, fnIndex }, R, F, compiled, errors);

function C(values: Record<string, number> = {}): Float64Array {
  return Float64Array.from(constKeys.map((k) => values[k] ?? state.constants[k]!.value));
}

/** Visible area −5..5 (computed: one screen more on each side), 100 px per unit. */
const RANGE: SampleRange = { x0: -15, x1: 15, y0: -15, y1: 15, sx: 100, sy: 100 };

function pts(key: string, consts?: Record<string, number>): [number, number, FeatureClass][] {
  set.run(C(consts), RANGE);
  return set.result(key).points.map((p) => [p.vx, p.vy, p.cls]);
}

function expectPts(key: string, want: [number, number, FeatureClass][], digits = 6, consts?: Record<string, number>): void {
  const got = pts(key, consts);
  expect(got.map((p) => p[2]), key).toEqual(want.map((p) => p[2]));
  got.forEach((p, i) => {
    expect(p[0], `${key}[${i}].x`).toBeCloseTo(want[i]![0], digits);
    if (Number.isNaN(want[i]![1])) expect(p[1]).toBeNaN();
    else expect(p[1], `${key}[${i}].y`).toBeCloseTo(want[i]![1], digits);
  });
}

describe("generated derivatives", () => {
  it("are compiled under their f@n names", () => {
    expect(errors).toEqual(new Map());
    expect(state.functions["cub@1"]).toBeUndefined();
    expect(compiled.get("cub@1")!.scalar(2, C())).toBe(9);
    expect(compiled.get("cub@2")!.scalar(2, C())).toBe(12);
    expect(compiled.get("q4@2")!.scalar(1, C())).toBe(12);
  });
});

describe("roots", () => {
  it("sign changes and touching roots", () => {
    expectPts("R", [[-Math.sqrt(3), 0, "Root"], [0, 0, "Root"], [Math.sqrt(3), 0, "Root"]]);
    expectPts("R4", [[0, 0, "TangentRoot"]], 3);
    expectPts("R2", [[-2, 0, "Root"], [1, 0, "TangentRoot"]], 5);
  });

  it("range slot with a constant", () => {
    expectPts("Rr", [[Math.sqrt(3), 0, "Root"]], 6, { c: 2 });
    expect(pts("Rr", { c: 1 })).toEqual([]);
  });

  it("poles and jumps are not roots", () => {
    expect(signChanges((x) => 1 / x, -1, 1, 401, 1e-9)).toEqual([]);
    expect(roots((x) => (x < 0 ? -1 : 1), -1, 1, 400, 1e-9)).toEqual([]);
  });
});

describe("extrema and inflection points (done criteria)", () => {
  it("a cubic's extrema and inflection point from generated f', f''", () => {
    expectPts("E", [[-1, 2, "Max"], [1, -2, "Min"]]);
    expectPts("I", [[0, 0, "Inflection"]]);
  });

  it("x^4 at 0: an extremum, not an inflection point", () => {
    expect(pts("I4")).toEqual([]);
    expectPts("E4", [[0, 0, "Min"]], 4);
  });

  it("filter word", () => {
    expectPts("Pm", [[1, -2, "Min"]]);
  });
});

describe("intersections, y-intercept, interval max / min", () => {
  it("with a curve and with y = expression", () => {
    expectPts("P", [[-2, -2, "Intersection"], [0, 0, "Intersection"], [2, 2, "Intersection"]]);
    // x³ − 3x − 2 = (x + 1)²(x − 2): touching at −1.
    expectPts("Q", [[-1, 2, "TangentIntersection"], [2, 2, "Intersection"]], 4);
  });

  it("y-intercept", () => {
    expectPts("Y", [[0, 0, "YIntercept"]]);
  });

  it("largest / smallest value on an interval, ends included", () => {
    expectPts("M", [[3, 18, "IntervalMax"]]);
    expectPts("Mm", [[-2, -2, "IntervalMin"], [1, -2, "IntervalMin"]]);
  });

  it("none past an asymptote in that direction, with the reason", () => {
    expectPts("Mi", []);
    expect(set.result("Mi").none).toContain("x = -3");
    expectPts("Mi2", []);
    expect(set.result("Mi2").none).toContain("작아짐");
    // The asymptote outside the interval does not matter.
    expectPts("Mi3", [[-2, 1, "IntervalMax"]]);
    expect(set.result("Mi3").none).toBeUndefined();
  });

  it("none when a hole's limit beats every value taken; a jump side taken counts", () => {
    expectPts("Mh", []);
    expect(set.result("Mh").none).toContain("닿지 않음");
    expectPts("Mj", [[0, 1, "IntervalMax"], [1, 1, "IntervalMax"]]);
  });

  it("slow growth (ln, 1/√x) counts as unbounded, not as a limit", () => {
    expectPts("Ml", []);
    expect(set.result("Ml").none).toContain("끝없이 작아짐");
    expectPts("Mr", []);
    expect(set.result("Mr").none).toContain("끝없이 커짐");
  });
});

describe("breaks", () => {
  it("hole and vertical asymptote", () => {
    expectPts("B", [[-2, NaN, "Asymptote"], [1, 2 + 1 / 3, "Hole"]], 5);
    expectPts("Bh", [[1, 2 + 1 / 3, "Hole"]], 5);
    set.run(C(), RANGE);
    expect(set.result("B").points[1]!.open).toBe(true);
  });

  it("cusp, domain endpoint, jump", () => {
    expectPts("Bc", [[1, 0, "Cusp"]]);
    expectPts("Bs", [[-3, 0, "Endpoint"]]);
    expectPts("Bj", [[0, -1, "Jump"], [0, 1, "Jump"]]);
    set.run(C(), RANGE);
    // f(0) = 1: the right side is filled, the left one open.
    expect(set.result("Bj").points.map((p) => p.open)).toEqual([true, false]);
  });

  it("floor / ceil / round / mod jumps at the steps", () => {
    // Each jump is two points (one per side): its x once.
    const xs = (key: string) => [...new Set(pts(key).filter(([, , c]) => c === "Jump").map(([x]) => Math.round(x * 1e6) / 1e6 + 0))];
    expectPts("Bf", [-2, -1, 0, 1, 2].flatMap((k) => [[k, k - 1, "Jump"], [k, k, "Jump"]] as [number, number, FeatureClass][]));
    // floor(x) takes the right side's value.
    expect(set.result("Bf").points.slice(0, 2).map((p) => p.open)).toEqual([true, false]);
    // Steps 0.01 apart: the one-sided probes stay between neighbouring steps.
    expect(xs("Bd")).toEqual([0.01, 0.02, 0.03]);
    expect(xs("Bo")).toEqual([0.5, 1.5]);
    expect(xs("Bm")).toEqual([0, 2]);
  });

  it("slow growth (ln, 1/√x) is an asymptote, not a hole", () => {
    expectPts("Bl", [[-3, NaN, "Asymptote"]]);
    expectPts("Br", [[0, NaN, "Asymptote"]]);
  });
});

describe("tangents", () => {
  it("parametric circle: horizontal and vertical, the closed curve's start counted once", () => {
    const got = pts("Tc").map(([x, y, c]) => [Math.round(x * 1e6) / 1e6 + 0, Math.round(y * 1e6) / 1e6 + 0, c]);
    expect(got).toEqual([
      [1, 0, "VerticalTangent"],
      [0, 1, "HorizontalTangent"],
      [-1, 0, "VerticalTangent"],
      [0, -1, "HorizontalTangent"],
    ]);
  });

  it("polar cardioid: horizontal tangents reported as (r, θ)", () => {
    // y = (1 + cos θ) sin θ: y' = 0 at θ = π/3, π, 5π/3 (π is the cusp: x' = 0 too, dropped).
    expectPts("Tp", [[1.5, Math.PI / 3, "HorizontalTangent"], [1.5, (5 * Math.PI) / 3, "HorizontalTangent"]], 5);
  });
});

describe("feature points as values", () => {
  it("one point: its coordinates; the tangent line at the minimum", () => {
    set.run(C(), RANGE);
    expect(compiled.get("T")!.scalar(5, C())).toBeCloseTo(-2, 9);
    expect(compiled.get("T")!.scalar(-3, C())).toBeCloseTo(-2, 9);
  });

  it("several points: undefined, with a warning on the user", () => {
    set.run(C(), RANGE);
    expect(compiled.get("U")!.scalar(1, C())).toBeNaN();
    expect(set.valueWarnings("U").map((w) => w.code)).toEqual(["W-feature-value"]);
    expect(set.valueWarnings("T")).toEqual([]);
  });
});
