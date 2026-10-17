// Numeric verification (stage 5c) against the state the Rust resolver produces.
// Fixture written by `cargo test -p dsl --test web_fixtures`.

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/verify.json";
import type { CheckResult } from "../src/generated/CheckResult";
import type { State } from "../src/generated/State";
import { compile, specsOf, type Scalar } from "../src/worker/codegen";
import { makeRuntime } from "../src/worker/numeric";
import { checkSpecsOf, constSets, numDeriv, POINTS, RANDOM_SETS, runCheck } from "../src/worker/verify";

const state = fixture as unknown as State;
const fns = Object.values(state.functions).map((f) => f!).sort((a, b) => a.order - b.order);
const specs = specsOf(state, fns);
const constKeys = Object.keys(state.constants).sort();
const constIndex = new Map(constKeys.map((k, i) => [k, i]));
const fnIndex = new Map(specs.map((f, i) => [f.key, i]));
const R = makeRuntime();
const F: Scalar[] = [];
specs.forEach((s, i) => {
  F[i] = compile(s, { constIndex, fnIndex }, R, F, false).table;
});
const current = Float64Array.from(constKeys.map((k) => state.constants[k]!.value));
const sets = constSets(state, constKeys, current, () => true);
const checks = checkSpecsOf(state);
const results = checks.map((c) => runCheck(c, sets, { env: { constIndex, fnIndex }, R, F, constKeys }));

function result(key: string, pair = 0): CheckResult {
  const r = results.find((c) => c.key === key && (c.pair === null || c.pair === pair));
  if (!r) throw new Error(`no check for ${key}`);
  return r;
}

describe("check list", () => {
  it("has one check per calc pair and per Claude f@n / ~f", () => {
    const keys = checks.map((c) => `${c.key}${c.pair === null ? "" : `#${c.pair}`}:${c.kind}`);
    expect(keys).toContain("C#0:Transform");
    expect(keys).toContain("C#1:Transform");
    expect(keys).toContain("Ci#0:Integral");
    expect(keys).toContain("Cm#0:Limit");
    expect(keys).toContain("cu@1:Deriv");
    expect(keys).toContain("~e:Inverse");
    // Plain functions get no check.
    expect(checks.some((c) => c.key === "f" || c.key === "cu")).toBe(false);
  });

  it("compares f@n with the highest lower order that exists", () => {
    const d2 = checks.find((c) => c.key === "cu@2")!;
    expect([d2.a, d2.order]).toEqual(["cu@1", 1]);
    const d3 = checks.find((c) => c.key === "du@3")!;
    expect([d3.a, d3.order]).toEqual(["du", 3]);
  });

  it("draws random constant sets inside the slider range", () => {
    expect(sets.length).toBe(1 + RANDOM_SETS);
    const a = state.constants["a"]!;
    for (const s of sets.slice(1)) expect(s[constIndex.get("a")!]).toBeGreaterThanOrEqual(a.min);
  });
});

describe("results", () => {
  it("transform: equal steps match, a wrong last step does not", () => {
    expect(result("C", 0).status).toBe("Match");
    expect(result("C", 1).status).toBe("Match");
    expect(result("C", 0).points).toBe(POINTS * sets.length);
    const bad = result("Cb");
    expect(bad.status).toBe("Mismatch");
    expect(bad.counter!.right - bad.counter!.left).toBeCloseTo(1, 9);
  });

  it("range restricts the points: sqrt(x^2) = x holds for x > 0 only", () => {
    expect(result("Cs").status).toBe("Match");
    const n = result("Cn");
    expect(n.status).toBe("Mismatch");
    expect(n.counter!.x).toBeLessThan(0);
  });

  it("one side undefined somewhere: conditional match", () => {
    const l = result("Cl");
    expect(l.status).toBe("Conditional");
    expect(l.one_sided).toBeGreaterThan(0);
  });

  it("integral: derivative of the antiderivative, constant ignored", () => {
    expect(result("Ci").status).toBe("Match");
    expect(result("Cw").status).toBe("Mismatch");
  });

  it("limit: not verified", () => {
    expect(result("Cm").status).toBe("NotVerified");
  });

  it("a wrong derivative supplied by Claude is a mismatch", () => {
    expect(result("cu@1").status).toBe("Match");
    expect(result("w@1").status).toBe("Mismatch");
    expect(result("du@3").status).toBe("Match");
  });

  it("random constant sets catch a mistake that holds at the current value", () => {
    // cu@2 = 6x is right only for a = 1 (the default).
    expect(state.constants["a"]!.value).toBe(1);
    const r = result("cu@2");
    expect(r.status).toBe("Mismatch");
    expect(r.counter!.consts[0]!.key).toBe("a");
    expect(r.counter!.consts[0]!.value).not.toBe(1);
  });

  it("inverse: f(~f(x)) = x where ~f is defined", () => {
    expect(result("~e").status).toBe("Match");
    expect(result("~q").status).toBe("Match");
    expect(result("~qb").status).toBe("Mismatch");
  });

  it("polar steps compare r over 0..2π", () => {
    expect(result("Cp").status).toBe("Match");
  });

  it("no point where both sides are defined: not verifiable", () => {
    expect(result("Cu").status).toBe("Unverifiable");
  });
});

describe("numeric derivative", () => {
  it("is accurate to the tolerances used", () => {
    expect(numDeriv(Math.sin, 0.7, 1)).toBeCloseTo(Math.cos(0.7), 10);
    expect(numDeriv(Math.exp, 1.3, 2)).toBeCloseTo(Math.exp(1.3), 6);
    expect(numDeriv((x) => x ** 4, 2, 3)).toBeCloseTo(48, 4);
  });
});
