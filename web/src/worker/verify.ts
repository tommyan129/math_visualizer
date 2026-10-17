// Numeric verification (stage 5c decisions, design.md "수치 검증"): Claude's results are
// compared with their source at fixed points. The server says what relates to what
// (`calc` steps, `f@n` / `~f` names); the browser evaluates.
// - calc transform: neighbouring steps equal;
// - calc integral: the numeric derivative of the second step equals the first;
// - f@n: equals the numeric derivative of the highest lower order that exists (f, f@m);
// - ~f: f(~f(x)) = x;
// - calc limit: not checked.

import type { CheckKind } from "../generated/CheckKind";
import type { CheckResult } from "../generated/CheckResult";
import type { Expr } from "../generated/Expr";
import type { State } from "../generated/State";
import { compileRegion, expr, type CodegenEnv, type Region, type Scalar } from "./codegen";
import type { Runtime } from "./numeric";

export interface CheckSpec {
  key: string;
  pair: number | null;
  kind: CheckKind;
  a: string;
  b: string;
  order: number;
  /** Input name the range is written in. */
  input: string;
  lo: Expr | null;
  hi: Expr | null;
  range: Expr | null;
  /** Interval used on a side the range leaves open. */
  fallback: [number, number];
  /** Break candidates of the compared steps: points keep away from their zeros. */
  candidates: Expr[];
  /** Constants either side uses (reported with a counterexample). */
  consts: string[];
}

/** Points per constant set (stage 5c decision). */
export const POINTS = 16;
/** Random constant sets besides the current values. */
export const RANDOM_SETS = 2;
/** Fewer compared points than this: not verifiable. */
const MIN_POINTS = 4;
/** Relative tolerance of an exact comparison, and of a first numeric derivative. */
const TOL_EXACT = 1e-9;
const TOL_NUMERIC = 1e-6;

/** Checks implied by the state: every calc pair, Claude's `f@n` and `~f`. */
export function checkSpecsOf(state: State): CheckSpec[] {
  const fns = state.functions;
  const out: CheckSpec[] = [];
  const constsOf = (...keys: string[]) => [...new Set(keys.flatMap((k) => fns[k]?.consts ?? []))].sort();
  const fallbackOf = (key: string): [number, number] => (fns[key]?.kind === "Polar" ? [0, 2 * Math.PI] : [-5, 5]);
  for (const f of Object.values(fns)) {
    if (!f) continue;
    const c = f.calc;
    if (c) {
      for (let i = 0; i + 1 < c.steps.length; i++) {
        const a = c.steps[i]!, b = c.steps[i + 1]!;
        const kind: CheckKind = c.kind === "Limit" ? "Limit" : c.kind === "Integral" && i === 0 ? "Integral" : "Transform";
        out.push({ key: f.key, pair: i, kind, a, b, order: kind === "Integral" ? 1 : 0, input: c.input.key, lo: c.lo, hi: c.hi, range: c.range, fallback: fallbackOf(a), candidates: c.candidates.map((x) => x.expr), consts: constsOf(a, b) });
      }
      continue;
    }
    const n = f.name.deriv;
    if (n > 0 && !f.name.inverse) {
      const base = f.key.replace(/@\d+$/, "");
      const g = fns[base];
      if (!g) continue;
      // Highest lower order that exists: Claude's f@m or a generated one.
      let m = n - 1;
      while (m > 0 && !fns[`${base}@${m}`] && g.derivs.length < m) m--;
      const a = m === 0 ? base : `${base}@${m}`;
      out.push({ key: f.key, pair: null, kind: "Deriv", a, b: f.key, order: n - m, input: f.inputs[0]!.key, lo: null, hi: null, range: null, fallback: [-5, 5], candidates: [], consts: constsOf(base, f.key) });
    } else if (f.name.inverse && n === 0) {
      const base = f.key.slice(1);
      if (!fns[base]) continue;
      out.push({ key: f.key, pair: null, kind: "Inverse", a: base, b: f.key, order: 0, input: f.inputs[0]!.key, lo: null, hi: null, range: null, fallback: [-5, 5], candidates: [], consts: constsOf(base, f.key) });
    }
  }
  return out;
}

/**
 * Constant sets: the current values, then `RANDOM_SETS` sets drawn inside each slider's
 * range (integers stay integers). Seeded, so the same state gives the same points.
 * `ok` rejects a set that breaks a slider constraint (split boundary order).
 */
export function constSets(state: State, keys: string[], current: Float64Array, ok: (C: Float64Array) => boolean): Float64Array[] {
  const sets = [current];
  const rand = mulberry32(0x5c);
  for (let s = 0; s < RANDOM_SETS; s++) {
    for (let tries = 0; tries < 20; tries++) {
      const C = Float64Array.from(keys, (k) => {
        const c = state.constants[k]!;
        const v = c.min + (c.max - c.min) * rand();
        return c.integer ? Math.round(v) : v;
      });
      if (ok(C)) {
        sets.push(C);
        break;
      }
    }
  }
  return sets;
}

function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** k-th central difference with Richardson extrapolation (h and h/2). */
export function numDeriv(f: (x: number) => number, x: number, k: number): number {
  const h = (k === 1 ? 1e-3 : 2e-2) * Math.max(1, Math.abs(x));
  const d = (h: number) => {
    let s = 0;
    for (let j = 0; j <= k; j++) s += (j % 2 ? -1 : 1) * binom(k, j) * f(x + (k / 2 - j) * h);
    return s / h ** k;
  };
  const d1 = d(h), d2 = d(h / 2);
  return (4 * d2 - d1) / 3;
}

function binom(n: number, k: number): number {
  let r = 1;
  for (let i = 1; i <= k; i++) r = (r * (n - k + i)) / i;
  return r;
}

/** Quasi-random point i in [a, b] (golden-ratio sequence). */
function point(i: number, a: number, b: number): number {
  const g = 0.6180339887498949;
  return a + (b - a) * ((0.5 + i * g) % 1);
}

export interface CheckEnv {
  env: CodegenEnv;
  R: Runtime;
  F: Scalar[];
  constKeys: string[];
}

/** Runs one check over every constant set. */
export function runCheck(spec: CheckSpec, sets: Float64Array[], ce: CheckEnv): CheckResult {
  const base: CheckResult = { key: spec.key, pair: spec.pair, kind: spec.kind, a: spec.a, b: spec.b, order: spec.order, status: "NotVerified", points: 0, one_sided: 0, counter: null };
  if (spec.kind === "Limit") return base;
  const { env, R, F } = ce;
  const ia = env.fnIndex.get(spec.a), ib = env.fnIndex.get(spec.b);
  if (ia === undefined || ib === undefined) return { ...base, status: "Unverifiable" };
  const make = (src: string) => new Function("R", "F", `"use strict";\nreturn ${src};`)(R, F);
  const constEnv: CodegenEnv = { ...env, axes: null, inputs: [] };
  const lo = spec.lo ? (make(`(C) => ${expr(spec.lo, constEnv)}`) as (C: Float64Array) => number) : null;
  const hi = spec.hi ? (make(`(C) => ${expr(spec.hi, constEnv)}`) as (C: Float64Array) => number) : null;
  const region: Region | null = spec.range ? compileRegion(spec.range, { ...env, axes: [spec.input, "\u0000"] }, R, F) : null;
  const cands = spec.candidates.map((e) => make(`(x, C) => ${expr(e, { ...env, axes: null })}`) as Scalar);
  const tol = spec.kind === "Transform" || spec.kind === "Inverse" ? TOL_EXACT : spec.order === 1 ? TOL_NUMERIC : 1e-4;
  // Exact side NaN where the numeric side is defined counts as one-sided; a numeric
  // derivative that cannot be taken (stencil past a break) skips the point.
  let points = 0, oneSided = 0;
  let counter: CheckResult["counter"] = null;
  for (const C of sets) {
    const fa = (x: number) => F[ia]!(x, C), fb = (x: number) => F[ib]!(x, C);
    let a = lo ? lo(C) : spec.fallback[0], b = hi ? hi(C) : spec.fallback[1];
    // A one-sided range (`x > 0`): the default width on the open side.
    if (lo && !hi) b = a + (spec.fallback[1] - spec.fallback[0]);
    if (!lo && hi) a = b - (spec.fallback[1] - spec.fallback[0]);
    if (!(a < b) || !Number.isFinite(a) || !Number.isFinite(b)) continue;
    const near = (b - a) * 1e-3;
    let taken = 0;
    for (let i = 0; i < POINTS * 4 && taken < POINTS; i++) {
      let x = point(i, a, b);
      // Keep away from candidate breaks: step past a sign change or zero nearby.
      for (let t = 0; t < 3 && cands.some((g) => crosses(g, x, near, C)); t++) x += 3 * near;
      if (x <= a || x >= b || (region && !region(x, 0, C))) continue;
      taken++;
      let left: number, right: number, numericSide: "left" | "right" | null = null;
      switch (spec.kind) {
        case "Transform":
          left = fa(x);
          right = fb(x);
          break;
        case "Integral":
          left = fa(x);
          right = numDeriv(fb, x, 1);
          numericSide = "right";
          break;
        case "Deriv":
          left = numDeriv(fa, x, spec.order);
          right = fb(x);
          numericSide = "left";
          break;
        case "Inverse": {
          const y = fb(x);
          if (!Number.isFinite(y)) continue;
          left = fa(y);
          right = x;
          break;
        }
      }
      const okL = Number.isFinite(left), okR = Number.isFinite(right);
      if (!okL && !okR) continue;
      if (!okL || !okR) {
        if ((numericSide === "left" && !okL) || (numericSide === "right" && !okR)) continue;
        oneSided++;
        continue;
      }
      points++;
      if (Math.abs(left - right) > tol * Math.max(Math.abs(left), Math.abs(right), 1) && !counter) {
        counter = { x, consts: spec.consts.map((k) => ({ key: k, value: C[ce.constKeys.indexOf(k)]! })), left, right };
      }
    }
  }
  const status = counter ? "Mismatch" : points < MIN_POINTS ? "Unverifiable" : oneSided > 0 ? "Conditional" : "Match";
  return { ...base, status, points, one_sided: oneSided, counter };
}

/** Does `g` vanish or change sign within `d` of `x`? */
function crosses(g: Scalar, x: number, d: number, C: Float64Array): boolean {
  const l = g(x - d, C), m = g(x, C), r = g(x + d, C);
  return m === 0 || Math.sign(l) !== Math.sign(r) || !Number.isFinite(m);
}
