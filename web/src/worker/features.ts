// Feature points (stage 5b): everything is "roots of some expression", the kinds differ
// only in what is solved and how a root is classified (design.md "특징점 찾는 방식").
// - roots / intersections: sign changes of f (or f − g), plus touching roots found as
//   local minima of |f| that reach zero;
// - extrema / inflection points: sign changes of f' / f'' (server derivatives or
//   Claude's f@n), so x⁴ at 0 is not an inflection point;
// - breaks: roots of the server's candidate expressions, classified by the values on
//   both sides (hole, vertical asymptote, jump, domain endpoint, cusp);
// - tangents of parametric / polar curves: sign changes of x'(t) and y'(t).

import type { FeatureClass } from "../generated/FeatureClass";
import type { FeatureSpec } from "../generated/FeatureSpec";
import { CodegenError, compileRegion, expr, type CodegenEnv, type Compiled, type FnSpec, type Region, type Scalar } from "./codegen";
import type { Runtime } from "./numeric";
import type { SampleRange } from "./sample";

/** Most points one definition reports (periodic functions over a wide range). */
export const MAX_FEATURE_POINTS = 200;

export interface FeaturePt {
  /** Screen (world) position; `y` is NaN for a vertical asymptote. */
  x: number;
  y: number;
  cls: FeatureClass;
  /** Drawn as an open circle: a value the function does not take (hole, jump side). */
  open: boolean;
  /** Value in the scene's axes (polar scenes: r, θ). */
  vx: number;
  vy: number;
}

export interface FeatureOut {
  points: FeaturePt[];
  capped: boolean;
}

/** One-input function at the current constants. */
type Fn1 = (x: number) => number;
/** Point-valued (parametric) function. */
type FnP = (t: number) => number[];

export interface CompiledFeature {
  spec: FeatureSpec;
  /** Run with the current constants and the computed x range (and visible y span). */
  run(C: Float64Array, x0: number, x1: number, yspan: number, pxPerUnit: number): FeatureOut;
}

const SCALAR_KINDS = new Set(["Curve", "Polar"]);

/**
 * Compiles a feature definition. `F` / `fnIndex` hold the target, its derivatives
 * (`f@1`, `f@2`: Claude's or generated) and the other curve's functions.
 */
export function compileFeature(spec: FeatureSpec, env: CodegenEnv, R: Runtime, F: Scalar[], targetKind: string, targetRange: ((C: Float64Array) => [number, number]) | null): CompiledFeature {
  const at = (key: string) => {
    const i = env.fnIndex.get(key);
    if (i === undefined) throw new CodegenError(`${key} 없음`);
    return i;
  };
  const make = (src: string) => new Function("R", "F", `"use strict";\nreturn ${src};`)(R, F);
  const scalarEnv: CodegenEnv = { ...env, axes: null };
  const constEnv: CodegenEnv = { ...env, axes: null, inputs: [] };
  const fx = (e: import("../generated/Expr").Expr) => make(`(x, C) => ${expr(e, scalarEnv)}`) as (x: number, C: Float64Array) => number;
  const fc = (e: import("../generated/Expr").Expr) => make(`(C) => ${expr(e, constEnv)}`) as (C: Float64Array) => number;

  const ti = at(spec.target);
  const order = derivOrder(spec.kind);
  const di = Array.from({ length: order }, (_, k) => at(`${spec.target}@${k + 1}`));
  const other = spec.other ? fx(spec.other) : null;
  const lo = spec.lo ? fc(spec.lo) : null;
  const hi = spec.hi ? fc(spec.hi) : null;
  // The range is an inequality over the target's input, mapped to the region's `x`.
  const region: Region | null = spec.range ? compileRegion(spec.range, { ...env, axes: [inputName(spec, targetKind), "\u0000"] }, R, F) : null;
  const cands = spec.candidates.map((c) => ({ cause: c.cause, fn: fx(c.expr) }));
  const parametric = targetKind === "Parametric";
  const polar = targetKind === "Polar";
  if (!parametric && !SCALAR_KINDS.has(targetKind)) throw new CodegenError("대상 종류");

  return {
    spec,
    run(C, x0, x1, yspan, pxPerUnit) {
      const f: Fn1 = (x) => F[ti]!(x, C);
      const d = di.map((i) => ((x: number) => F[i]!(x, C)) as Fn1);
      let a = lo ? lo(C) : x0, b = hi ? hi(C) : x1;
      if (parametric || polar) {
        // The parameter range of the curve, narrowed by the range slot.
        const [t0, t1] = targetRange ? targetRange(C) : [0, 2 * Math.PI];
        a = lo ? Math.max(a, t0) : t0;
        b = hi ? Math.min(b, t1) : t1;
      }
      if (!(a <= b) || !Number.isFinite(a) || !Number.isFinite(b)) return { points: [], capped: false };
      const keep = (x: number) => !region || region(x, 0, C);
      const n = Math.max(400, Math.min(20000, Math.ceil((b - a) * pxPerUnit)));
      const tol = 1e-7 * Math.max(yspan, 1e-12);
      let pts: FeaturePt[];
      const pt = (x: number, y: number, cls: FeatureClass): FeaturePt => ({ x, y, cls, open: false, vx: x, vy: y });
      switch (spec.kind) {
        case "Roots":
          pts = roots(f, a, b, n, tol).map(([x, touch]) => pt(x, 0, touch ? "TangentRoot" : "Root"));
          break;
        case "Intersect": {
          const g = (x: number) => other!(x, C);
          pts = roots((x) => f(x) - g(x), a, b, n, tol).map(([x, touch]) => pt(x, f(x), touch ? "TangentIntersection" : "Intersection"));
          break;
        }
        case "Yint":
          pts = a <= 0 && 0 <= b && Number.isFinite(f(0)) ? [pt(0, f(0), "YIntercept")] : [];
          break;
        case "Extrema":
          pts = signChanges(d[0]!, a, b, n, tol)
            .filter(([x]) => Number.isFinite(f(x)))
            .map(([x, dir]) => pt(x, f(x), dir < 0 ? "Max" : "Min"));
          if (spec.filter) pts = pts.filter((p) => p.cls === (spec.filter === "max" ? "Max" : "Min"));
          break;
        case "Inflections":
          pts = signChanges(d[1]!, a, b, n, tol).filter(([x]) => Number.isFinite(f(x))).map(([x]) => pt(x, f(x), "Inflection"));
          break;
        case "Argmax":
        case "Argmin":
          pts = intervalExtreme(f, d[0]!, a, b, n, tol, keep, spec.kind === "Argmax");
          break;
        case "Breaks":
          pts = breaks(f, cands.map((c) => (x: number) => c.fn(x, C)), a, b, n, yspan);
          if (spec.filter) pts = pts.filter((p) => breakFilter(p.cls) === spec.filter);
          break;
        case "Tangents": {
          const P: FnP = parametric ? (t) => F[ti]!(t, C) as unknown as number[] : (t) => polarPoint(f(t), t);
          const dP: FnP = parametric
            ? (t) => F[di[0]!]!(t, C) as unknown as number[]
            : (t) => {
                const r = f(t), dr = d[0]!(t);
                return [dr * Math.cos(t) - r * Math.sin(t), dr * Math.sin(t) + r * Math.cos(t)];
              };
          const found: [number, FeaturePt][] = [];
          const add = (comp: 0 | 1, cls: FeatureClass) => {
            const g = (t: number) => dP(t)[comp]!;
            // A zero at either end of the parameter range counts (closed curves start on one).
            const ends = [a, b].filter((t) => Math.abs(g(t)) <= tol).map((t) => [t, 0] as [number, number]);
            for (const [t] of [...signChanges(g, a, b, n, tol), ...ends]) {
              const q = P(t), dq = dP(t);
              // Both derivatives zero: a singular point, not a tangent direction.
              if (!q.every(Number.isFinite) || Math.abs(dq[1 - comp]!) < tol || !keep(t)) continue;
              found.push([t, { x: q[0]!, y: q[1]!, cls, open: false, vx: polar ? f(t) : q[0]!, vy: polar ? t : q[1]! }]);
            }
          };
          if (spec.filter !== "vertical") add(1, "HorizontalTangent");
          if (spec.filter !== "horizontal") add(0, "VerticalTangent");
          // In parameter order; a closed curve's start and end are one point.
          found.sort((p, q) => p[0] - q[0]);
          const uniq = found.filter(([, p], i) => !found.slice(0, i).some(([, q]) => q.cls === p.cls && Math.hypot(q.x - p.x, q.y - p.y) <= 1e-9 * Math.max(1, Math.hypot(p.x, p.y))));
          return cap(uniq.map(([, p]) => p));
        }
        default:
          pts = [];
      }
      return cap(pts.filter((p) => keep(p.x)));
    },
  };
}

function polarPoint(r: number, t: number): number[] {
  return [r * Math.cos(t), r * Math.sin(t)];
}

function cap(pts: FeaturePt[]): FeatureOut {
  return pts.length > MAX_FEATURE_POINTS ? { points: pts.slice(0, MAX_FEATURE_POINTS), capped: true } : { points: pts, capped: false };
}

function derivOrder(kind: FeatureSpec["kind"]): number {
  switch (kind) {
    case "Extrema":
    case "Argmax":
    case "Argmin":
    case "Tangents":
      return 1;
    case "Inflections":
      return 2;
    default:
      return 0;
  }
}

/** The target's input name, which the range slot is written in. */
function inputName(spec: FeatureSpec, targetKind: string): string {
  // Curves are placed by (input, output); polar scenes by (r, θ) with θ the input.
  if (targetKind === "Polar") return spec.axes[1]?.key ?? "\\theta";
  if (targetKind === "Parametric") return rangeInput(spec) ?? "t";
  return spec.axes[0]?.key ?? "x";
}

/** Input name used in a parametric range slot (the only input name in it). */
function rangeInput(spec: FeatureSpec): string | null {
  let name: string | null = null;
  const walk = (e: import("../generated/Expr").Expr) => {
    if (e.type === "Name" && e.kind === "Input") name = e.name.key;
    for (const c of childrenOf(e)) walk(c);
  };
  if (spec.range) walk(spec.range);
  return name;
}

function childrenOf(e: import("../generated/Expr").Expr): import("../generated/Expr").Expr[] {
  switch (e.type) {
    case "Compare":
      return [e.first, ...e.rest.map((l) => l.rhs)];
    case "And":
    case "Or":
      return e.args;
    case "Not":
      return [e.arg];
    case "Sum":
      return e.terms.map((t) => t.expr);
    case "Product":
      return e.factors;
    case "Div":
      return [e.num, e.den];
    case "Pow":
      return [e.base, e.exp];
    case "Neg":
      return [e.arg];
    case "Paren":
      return [e.inner];
    case "Call":
      return e.args;
    default:
      return [];
  }
}

function breakFilter(cls: FeatureClass): string {
  switch (cls) {
    case "Hole":
    case "Filled":
      return "hole";
    case "Asymptote":
      return "asymptote";
    case "Jump":
      return "jump";
    case "Endpoint":
      return "endpoint";
    case "Cusp":
      return "cusp";
    default:
      return "";
  }
}

// ----- root finding -------------------------------------------------------

/** Bisection on a bracket with a sign change; returns the midpoint of the last bracket. */
function bisect(g: Fn1, a: number, b: number, ga: number): number {
  for (let i = 0; i < 80; i++) {
    const m = 0.5 * (a + b);
    if (m <= a || m >= b) break;
    const gm = g(m);
    if (gm === 0) return m;
    if (!Number.isFinite(gm)) {
      // Undefined inside the bracket: keep the side that stays defined near a.
      b = m;
      continue;
    }
    if (Math.sign(gm) === Math.sign(ga)) {
      a = m;
      ga = gm;
    } else b = m;
  }
  return 0.5 * (a + b);
}

/**
 * Sign changes of g on a grid of n cells, refined by bisection. Returns [x, direction]
 * (direction −1: + to −, +1: − to +). A change across a pole or jump (|g| does not
 * shrink near the crossing) is not a root.
 */
export function signChanges(g: Fn1, a: number, b: number, n: number, tol: number): [number, number][] {
  const out: [number, number][] = [];
  const h = (b - a) / n;
  let px = a, pg = g(a);
  // A sample exactly at zero: decided by its neighbours.
  for (let i = 1; i <= n; i++) {
    const x = i === n ? b : a + i * h;
    const gx = g(x);
    if (Number.isFinite(pg) && Number.isFinite(gx)) {
      if (pg === 0) {
        // Handled when it was the right end (below).
      } else if (gx === 0) {
        const nx = Math.min(b, x + h), gn = g(nx);
        if (Number.isFinite(gn) && gn !== 0 && Math.sign(gn) !== Math.sign(pg)) out.push([x, pg > 0 ? -1 : 1]);
      } else if (Math.sign(pg) !== Math.sign(gx)) {
        const r = bisect(g, px, x, pg);
        const gr = Math.abs(g(r));
        // Pole / jump: the value at the crossing stays as large as at the ends.
        if (gr <= Math.max(tol, 1e-3 * Math.min(Math.abs(pg), Math.abs(gx)))) out.push([r, pg > 0 ? -1 : 1]);
      }
    }
    px = x;
    pg = gx;
  }
  return out;
}

/** Golden-section minimum of |g| on [a, b]. */
function minAbs(g: Fn1, a: number, b: number): number {
  const phi = (Math.sqrt(5) - 1) / 2;
  let c = b - phi * (b - a), d = a + phi * (b - a);
  let gc = Math.abs(g(c)), gd = Math.abs(g(d));
  for (let i = 0; i < 100 && b - a > 1e-15 * Math.max(1, Math.abs(a)); i++) {
    if (gc < gd) {
      b = d;
      d = c;
      gd = gc;
      c = b - phi * (b - a);
      gc = Math.abs(g(c));
    } else {
      a = c;
      c = d;
      gc = gd;
      d = a + phi * (b - a);
      gd = Math.abs(g(d));
    }
  }
  return 0.5 * (a + b);
}

/**
 * Roots of g: sign changes (false) and touching roots without a sign change (true),
 * found as local minima of |g| that reach zero within `tol`. Sorted, duplicates merged.
 */
export function roots(g: Fn1, a: number, b: number, n: number, tol: number): [number, boolean][] {
  const out: [number, boolean][] = signChanges(g, a, b, n, tol).map(([x]) => [x, false]);
  const h = (b - a) / n;
  const xs = Array.from({ length: n + 1 }, (_, i) => (i === n ? b : a + i * h));
  const gs = xs.map(g);
  for (let i = 1; i < n; i++) {
    const l = gs[i - 1]!, m = gs[i]!, r = gs[i + 1]!;
    if (![l, m, r].every(Number.isFinite)) continue;
    // Same sign on both sides and |g| dips in the middle.
    if (Math.sign(l) !== Math.sign(m) || Math.sign(r) !== Math.sign(m) || m === 0) continue;
    if (!(Math.abs(m) <= Math.abs(l) && Math.abs(m) < Math.abs(r))) continue;
    const x = minAbs(g, xs[i - 1]!, xs[i + 1]!);
    if (Math.abs(g(x)) <= tol) out.push([x, true]);
  }
  // Exact zero samples with no sign change are touching roots too.
  for (let i = 1; i < n; i++) {
    if (gs[i] === 0 && Math.sign(gs[i - 1]!) === Math.sign(gs[i + 1]!) && gs[i - 1] !== 0) out.push([xs[i]!, true]);
  }
  out.sort((p, q) => p[0] - q[0]);
  return dedupe(out, (b - a) * 1e-9);
}

function dedupe<T extends [number, ...unknown[]]>(xs: T[], eps: number): T[] {
  const out: T[] = [];
  for (const x of xs) if (!out.length || Math.abs(x[0] - out[out.length - 1]![0]) > eps) out.push(x);
  return out;
}

/** Largest (or smallest) value on [a, b]: critical points and the ends that are inside. */
function intervalExtreme(f: Fn1, d1: Fn1, a: number, b: number, n: number, tol: number, keep: (x: number) => boolean, max: boolean): FeaturePt[] {
  const xs = signChanges(d1, a, b, n, tol).map(([x]) => x);
  for (const e of [a, b]) if (keep(e)) xs.push(e);
  const vals = xs.map((x) => [x, f(x)] as const).filter(([, y]) => Number.isFinite(y));
  if (vals.length === 0) return [];
  const best = vals.reduce((m, [, y]) => (max ? Math.max(m, y) : Math.min(m, y)), max ? -Infinity : Infinity);
  const eps = 1e-9 * Math.max(1, Math.abs(best));
  const cls: FeatureClass = max ? "IntervalMax" : "IntervalMin";
  const top = vals.filter(([, y]) => Math.abs(y - best) <= eps).map(([x, y]) => [x, y] as [number, number]);
  return dedupe(top.sort((p, q) => p[0] - q[0]), (b - a) * 1e-9).map(([x, y]) => ({ x, y, cls, open: false, vx: x, vy: y }));
}

/**
 * Breaks at the roots of the candidate expressions, classified from f near each root:
 * one side undefined → domain endpoint; growing without bound → vertical asymptote;
 * different one-sided limits → jump; a limit f does not take → hole; different
 * one-sided slopes → cusp. A root where none of these holds is not a break.
 */
function breaks(f: Fn1, cands: Fn1[], a: number, b: number, n: number, yspan: number): FeaturePt[] {
  const w = b - a;
  const xs: number[] = [];
  for (const g of cands) {
    // Candidates are continuous expressions (denominators, arguments), so touching roots
    // count (x² in a denominator); the tolerance is relative to the candidate's size.
    const size = Math.max(1, Math.abs(g(a)), Math.abs(g(b)), Math.abs(g(0.5 * (a + b))));
    for (const [x] of roots(g, a, b, n, 1e-9 * size)) xs.push(x);
  }
  xs.sort((p, q) => p - q);
  const out: FeaturePt[] = [];
  const tolY = 1e-4 * Math.max(yspan, 1e-12);
  for (const [x0] of dedupe(xs.map((x) => [x] as [number]), w * 1e-9)) {
    const s = Math.max(Math.abs(x0), w, 1e-12);
    const near = 1e-7 * s, far = 1e-4 * s;
    const L = f(x0 - near), R = f(x0 + near), fx = f(x0);
    const L2 = f(x0 - far), R2 = f(x0 + far);
    const pt = (y: number, cls: FeatureClass, open = false): FeaturePt => ({ x: x0, y, cls, open, vx: x0, vy: y });
    const lOk = Number.isFinite(L), rOk = Number.isFinite(R);
    if (!lOk && !rOk) continue;
    const blows = (v: number, v2: number) => Number.isFinite(v) && Math.abs(v) > 100 * Math.max(1, Math.abs(v2)) && Math.abs(v) > 1e3 * tolY;
    if (blows(L, L2) || blows(R, R2)) {
      out.push(pt(NaN, "Asymptote"));
      continue;
    }
    if (lOk !== rOk) {
      out.push(Number.isFinite(fx) ? pt(fx, "Endpoint") : pt(lOk ? L : R, "Endpoint", true));
      continue;
    }
    if (Math.abs(L - R) > tolY) {
      // Jump: each side's limit, open where f does not take that value.
      for (const v of [L, R]) out.push(pt(v, "Jump", !(Number.isFinite(fx) && Math.abs(fx - v) <= tolY)));
      if (Number.isFinite(fx) && Math.abs(fx - L) > tolY && Math.abs(fx - R) > tolY) out.push(pt(fx, "Filled"));
      continue;
    }
    const M = 0.5 * (L + R);
    if (!Number.isFinite(fx)) {
      out.push(pt(M, "Hole", true));
      continue;
    }
    if (Math.abs(fx - M) > tolY) {
      out.push(pt(M, "Hole", true), pt(fx, "Filled"));
      continue;
    }
    const h = 1e-5 * s;
    const sl = (fx - f(x0 - h)) / h, sr = (f(x0 + h) - fx) / h;
    if (Number.isFinite(sl) && Number.isFinite(sr) && Math.abs(sl - sr) > 1e-2 * Math.max(1, Math.abs(sl), Math.abs(sr))) out.push(pt(fx, "Cusp"));
  }
  return out;
}

/**
 * All feature definitions of a workspace: compiled in dependency order (a feature whose
 * target uses another feature's point runs after it), run once per constants and range,
 * and the value table entries (`P` in `P_[1]`) that read the results.
 */
export class FeatureSet {
  private order: { key: string; cf: CompiledFeature }[] = [];
  private out = new Map<string, FeatureOut>();
  private stamp = "";
  /** Definitions that use feature points as values, and which ones. */
  private users = new Map<string, string[]>();

  set(fns: FnSpec[], env: CodegenEnv, R: Runtime, F: Scalar[], compiled: Map<string, Compiled>, errors: Map<string, string>): void {
    const byKey = new Map(fns.map((f) => [f.key, f]));
    const isFeature = (k: string) => byKey.get(k)?.kind === "Feature";
    // Features reachable through a definition's references (not through other features).
    const reach = (k: string): string[] => {
      const found = new Set<string>();
      const seen = new Set<string>();
      const stack = [...(byKey.get(k)?.calls ?? [])];
      while (stack.length) {
        const c = stack.pop()!;
        if (seen.has(c)) continue;
        seen.add(c);
        if (isFeature(c)) found.add(c);
        else stack.push(...(byKey.get(c)?.calls ?? []));
      }
      return [...found];
    };
    this.users = new Map();
    for (const f of fns) {
      if (f.kind === "Feature") continue;
      const used = reach(f.key);
      if (used.length) this.users.set(f.key, used);
    }
    const keys: string[] = [];
    const done = new Set<string>();
    const visit = (k: string) => {
      if (done.has(k)) return;
      done.add(k);
      reach(k).forEach(visit);
      keys.push(k);
    };
    fns.filter((f) => f.kind === "Feature").forEach((f) => visit(f.key));
    this.order = [];
    for (const key of keys) {
      const spec = byKey.get(key)!;
      const i = env.fnIndex.get(key)!;
      // Value table entry `(C) => [x, y]`: the result of the last run.
      F[i] = (() => this.value(key)) as unknown as Scalar;
      try {
        const fs = spec.feature!;
        const target = byKey.get(fs.target);
        const tc = compiled.get(fs.target);
        if (!target || !tc) throw new CodegenError(`대상 ${fs.target} 없음`);
        this.order.push({ key, cf: compileFeature(fs, env, R, F, target.kind, tc.trange ?? null) });
      } catch (e) {
        errors.set(key, e instanceof CodegenError ? e.message : String(e));
      }
    }
    this.stamp = "";
  }

  /** Finds every feature point at these constants and computed range (once per pair). */
  run(C: Float64Array, range: SampleRange): void {
    const stamp = `${Array.from(C).join(",")}|${range.x0},${range.x1},${range.y0},${range.y1},${range.sx}`;
    if (stamp === this.stamp) return;
    this.stamp = stamp;
    this.out = new Map();
    // The computed range is the visible area plus one screen on each side.
    const yspan = (range.y1 - range.y0) / 3;
    for (const { key, cf } of this.order) {
      let o: FeatureOut;
      try {
        o = cf.run(C, range.x0, range.x1, yspan, range.sx);
      } catch {
        o = { points: [], capped: false };
      }
      this.out.set(key, o);
    }
  }

  result(key: string): FeatureOut {
    return this.out.get(key) ?? { points: [], capped: false };
  }

  /** The point in the scene's axes when there is exactly one; otherwise undefined. */
  value(key: string): number[] {
    const o = this.out.get(key);
    const p = o && o.points.length === 1 ? o.points[0]! : null;
    return p && Number.isFinite(p.vx) && Number.isFinite(p.vy) ? [p.vx, p.vy] : [NaN, NaN];
  }

  /** For a definition using feature points whose count is not one. */
  valueWarnings(key: string): { source: string; code: string; message: string }[] {
    return (this.users.get(key) ?? []).flatMap((p) => {
      const n = this.out.get(p)?.points.length ?? 0;
      return n === 1 ? [] : [{ source: key, code: "W-feature-value", message: `특징점 ${p}가 지금 값에서 ${n}개라 (하나여야 함) 그 값을 쓰는 곳은 정의 안 됨` }];
    });
  }
}
