// AST → JS code generation (design.md "Evaluator").
//
// Only validated node kinds are assembled. No text from the AST reaches the code:
// constants are indices into `C`, user functions are indices into `F`, index
// variables are numbered locals, and numbers are re-emitted from parsed values.

import type { DefKind } from "../generated/DefKind";
import type { Expr } from "../generated/Expr";
import type { FeatureSpec } from "../generated/FeatureSpec";
import type { Function as Fn } from "../generated/Function";
import type { State } from "../generated/State";
import type { Ty } from "../generated/Ty";
import type { Runtime } from "./numeric";

export type Scalar = (x: number, C: Float64Array) => number;
/** Inequality region over (x, y); `y` is ignored for x-only conditions. */
export type Region = (x: number, y: number, C: Float64Array) => boolean;
export type Batch = (xs: Float64Array, out: Float64Array, C: Float64Array) => void;

/** Vectors and matrices: row-major arrays (sizes come from the node types). */
export type Vec = (C: Float64Array) => number[];

export interface FnSpec {
  key: string;
  version: number;
  kind: DefKind;
  /** Input names (curves: one; matrix functions: one or more; others: none). */
  inputs: string[];
  body: Expr;
  consts: string[];
  calls: string[];
  /** Parameter range of a parametric / polar function. */
  range: { lo: Expr; hi: Expr } | null;
  /** `family(...; lo <= k <= hi)`. */
  family: { index: string; lo: Expr; hi: Expr; lo_strict: boolean; hi_strict: boolean } | null;
  /** Feature-point definition (stage 5b). */
  feature: FeatureSpec | null;
}

export function specOf(f: Fn): FnSpec {
  const range = f.range ? { lo: f.range.lo, hi: f.range.hi } : null;
  const family = f.family ? { index: f.family.index.key, lo: f.family.lo, hi: f.family.hi, lo_strict: f.family.lo_strict, hi_strict: f.family.hi_strict } : null;
  return { key: f.key, version: f.version, kind: f.kind, inputs: f.inputs.map((i) => i.key), body: f.body, consts: f.consts, calls: f.calls, range, family, feature: f.feature };
}

/**
 * Every definition plus the server-generated derivatives `f@n` that Claude has not
 * defined (design.md "f@n"): same kind, inputs and constants as f, never drawn.
 */
export function specsOf(state: State, fns: Fn[]): FnSpec[] {
  const out = fns.map(specOf);
  for (const f of fns) {
    f.derivs.forEach((body, i) => {
      const key = `${f.key}@${i + 1}`;
      if (state.functions[key]) return;
      out.push({ ...specOf(f), key, body, family: null, feature: null, calls: [...f.calls, f.key] });
    });
  }
  return out;
}

/** A point of a parametric / polar curve at parameter `t` (`[NaN, NaN]` where undefined). */
export type Param = (t: number, C: Float64Array) => number[];
/** Implicit curve `lhs - rhs` over the scene's horizontal and vertical axis. */
export type Implicit = (x: number, y: number, C: Float64Array) => number;

export interface Compiled {
  key: string;
  version: number;
  kind: DefKind;
  scalar: Scalar;
  batch: Batch;
  /**
   * Entry in the shared table `F`: the scalar function of a curve, `(x0, …, C) => array`
   * of a matrix function, `(C) => array` of a vector / matrix value.
   */
  table: Scalar;
  /** Span vectors, or an arrow's vector and start point. */
  vecs: Vec[];
  /** Defined only at integer x (x appears in a sum/prod bound, here or in a callee). */
  discrete: boolean;
  /** Parametric and polar curves (polar: the point r(θ)·(cos θ, sin θ)). */
  param?: Param;
  /** Parameter range at the current constants; polar default 0..2π. */
  trange?: (C: Float64Array) => [number, number];
  /** Equations. */
  implicit?: Implicit;
  /** `family`: members per integer k, and the index bounds at the current constants. */
  family?: { at: (k: number) => DrawnMembers; krange: (C: Float64Array) => [number, number]; loStrict: boolean; hiStrict: boolean };
}

export interface CodegenEnv {
  /** Global constant key → index in `C`. */
  constIndex: Map<string, number>;
  /** User function key → index in `F`. */
  fnIndex: Map<string, number>;
  /**
   * Regions: the scene's horizontal and vertical axis names (`["t", "y"]`), mapped to
   * the `x` and `y` parameters. Default `["x", "y"]`. `null` in a function body, whose
   * single input is always `x`.
   */
  axes?: [string, string] | null;
  /** Matrix function inputs, mapped to the parameters `x0, x1, …`. */
  inputs?: string[];
}

/** (rows, columns) of a vector / matrix type. */
export function dims(ty: Ty): [number, number] {
  if (ty.type === "Vector") return [ty.n, 1];
  if (ty.type === "Matrix") return [ty.m, ty.n];
  throw new CodegenError(`not a vector or matrix`);
}

const isArray = (ty: Ty) => ty.type === "Vector" || ty.type === "Matrix";

function int(v: number): string {
  if (!Number.isInteger(v) || v < 0 || v > 1_000_000) throw new CodegenError("bad size");
  return String(v);
}

const BUILTIN_1 = new Set([
  "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "exp", "ln", "log10", "log2",
  "sqrt", "abs", "sign", "floor", "ceil", "round", "deg", "gamma",
]);
const BUILTIN_2 = new Set(["log", "mod"]);

const NUMBER_RE = /^[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?$/;

export class CodegenError extends Error {}

function num(text: string): string {
  if (!NUMBER_RE.test(text)) throw new CodegenError(`invalid number literal`);
  const v = Number(text);
  if (!Number.isFinite(v)) throw new CodegenError(`number out of range`);
  // Re-emit from the parsed value; `1e-3` → `0.001`.
  return `(${String(v)})`;
}

function gcd(a: number, b: number): number {
  while (b) [a, b] = [b, a % b];
  return Math.abs(a);
}

/** `p/q` written as integer literals (possibly in parentheses or negated) → reduced [p, q]. */
export function rationalExponent(e: Expr): [number, number] | null {
  let sign = 1;
  let cur: Expr = e;
  for (;;) {
    if (cur.type === "Paren") cur = cur.inner;
    else if (cur.type === "Neg") {
      sign = -sign;
      cur = cur.arg;
    } else break;
  }
  if (cur.type !== "Div") return null;
  const intOf = (x: Expr): number | null => {
    let s = 1;
    while (x.type === "Paren" || x.type === "Neg") {
      if (x.type === "Neg") s = -s;
      x = x.type === "Paren" ? x.inner : x.arg;
    }
    return x.type === "Num" && x.int ? s * Number(x.text) : null;
  };
  const p = intOf(cur.num);
  const q = intOf(cur.den);
  if (p === null || q === null || q === 0) return null;
  const g = gcd(p, q) || 1;
  let rp = (sign * p) / g;
  let rq = q / g;
  if (rq < 0) {
    rp = -rp;
    rq = -rq;
  }
  return [rp, rq];
}

/** Does x appear inside a sum/prod bound? */
function boundUsesInput(e: Expr, inBound = false): boolean {
  switch (e.type) {
    case "Name":
      return inBound && e.kind === "Input";
    case "BigOp":
      return boundUsesInput(e.body, inBound) || boundUsesInput(e.lo, true) || boundUsesInput(e.hi, true);
    default:
      return children(e).some((c) => boundUsesInput(c, inBound));
  }
}

export function children(e: Expr): Expr[] {
  switch (e.type) {
    case "Sum":
      return e.terms.map((t) => t.expr);
    case "Product":
      return e.factors;
    case "Div":
      return [e.num, e.den];
    case "Pow":
      return [e.base, e.exp];
    case "Neg":
    case "Factorial":
    case "Not":
      return [e.arg];
    case "Paren":
      return [e.inner];
    case "Call":
      return e.args;
    case "BigOp":
      return [e.body, e.lo, e.hi];
    case "Compare":
      return [e.first, ...e.rest.map((l) => l.rhs)];
    case "And":
    case "Or":
      return e.args;
    case "Tuple":
    case "List":
      return e.items;
    case "Index":
      return [e.base, ...e.indices];
    case "Address":
      return e.step.type === "Term" ? [e.base, e.step.pattern] : [e.base];
    case "Split":
      return [e.subject, ...e.at, ...e.pieces.flatMap((p) => [p.value, p.cond])];
    default:
      return [];
  }
}

/** JS expression for `e`. `indices`: enclosing sum/prod index keys, innermost last. */
export function expr(e: Expr, env: CodegenEnv, indices: string[] = []): string {
  const go = (c: Expr) => expr(c, env, indices);
  switch (e.type) {
    case "Num":
      return num(e.text);
    case "Name":
      switch (e.kind) {
        case "Input": {
          if (env.inputs) {
            const i = env.inputs.indexOf(e.name.key);
            if (i < 0) throw new CodegenError("unknown input");
            return `x${i}`;
          }
          // Regions use both axes of the scene; a function body has one input.
          if (env.axes === null) return "x";
          const [, v] = env.axes ?? ["x", "y"];
          return e.name.key === v ? "y" : "x";
        }
        case "Index": {
          const i = indices.lastIndexOf(e.name.key);
          if (i < 0) throw new CodegenError("unbound index");
          return `k${i}`;
        }
        case "Constant": {
          const i = env.constIndex.get(e.name.key);
          if (i === undefined) throw new CodegenError("unknown constant");
          return `C[${i}]`;
        }
        case "Value": {
          const i = env.fnIndex.get(e.name.key);
          if (i === undefined) throw new CodegenError("unknown value");
          return `F[${i}](C)`;
        }
        case "SplitName":
          return "s0";
        case "Reserved":
          if (e.name.key === "\\pi") return "R.PI";
          if (e.name.key === "\\e") return "R.E";
          throw new CodegenError("unsupported reserved value");
        default:
          throw new CodegenError("unresolved name");
      }
    case "Sum":
      if (isArray(e.ty)) {
        let acc = "";
        e.terms.forEach((t, i) => {
          const v = go(t.expr);
          if (i === 0) acc = t.sign === "Minus" ? `R.mscale(-1, ${v})` : v;
          else acc = `R.${t.sign === "Minus" ? "msub" : "madd"}(${acc}, ${v})`;
        });
        return acc;
      }
      return `(${e.terms.map((t, i) => (t.sign === "Minus" ? " - " : i === 0 ? "" : " + ") + go(t.expr)).join("")})`;
    case "Product": {
      const arr = e.factors.filter((f) => isArray(f.ty));
      if (arr.length === 1) {
        const scalars = e.factors.filter((f) => !isArray(f.ty)).map(go);
        return `R.mscale(${scalars.length ? `(${scalars.join(" * ")})` : "1"}, ${go(arr[0]!)})`;
      }
      if (arr.length > 1) throw new CodegenError("matrix product");
      return `(${e.factors.map(go).join(" * ")})`;
    }
    case "Div":
      if (isArray(e.num.ty)) return `R.mscale(1 / ${go(e.den)}, ${go(e.num)})`;
      return `(${go(e.num)} / ${go(e.den)})`;
    case "Pow": {
      const rat = rationalExponent(e.exp);
      if (rat) return `R.powRat(${go(e.base)}, ${rat[0]}, ${rat[1]})`;
      return `R.pow(${go(e.base)}, ${go(e.exp)})`;
    }
    case "Neg":
      if (isArray(e.ty)) return `R.mscale(-1, ${go(e.arg)})`;
      return `(-${go(e.arg)})`;
    case "Factorial":
      return `R.fact(${go(e.arg)})`;
    case "Paren":
      return go(e.inner);
    case "Call": {
      const key = e.callee.key;
      if (e.kind === "Builtin") {
        if (BUILTIN_1.has(key) && e.args.length === 1) return `R.${key}(${go(e.args[0]!)})`;
        if (BUILTIN_2.has(key) && e.args.length === 2) return `R.${key}(${go(e.args[0]!)}, ${go(e.args[1]!)})`;
        return linalg(e, go);
      }
      if (e.kind === "Function") {
        const i = env.fnIndex.get(key);
        if (i === undefined || e.args.length === 0) throw new CodegenError("unknown function");
        return `F[${i}](${e.args.map(go).join(", ")}, C)`;
      }
      throw new CodegenError("unresolved call");
    }
    case "BigOp": {
      const k = `k${indices.length}`;
      const body = expr(e.body, env, [...indices, e.index.key]);
      const op = e.op === "Sum" ? "sum" : "prod";
      return `R.${op}(${go(e.lo)}, ${go(e.hi)}, ${e.lo_strict}, ${e.hi_strict}, (${k}) => ${body})`;
    }
    case "Compare": {
      // Chains compare each neighbor pair: a < b <= c → (a < b) && (b <= c).
      const parts: string[] = [];
      let lhs = go(e.first);
      for (const l of e.rest) {
        const rhs = go(l.rhs);
        parts.push(`(${lhs} ${CMP[l.op]} ${rhs})`);
        lhs = rhs;
      }
      return `(${parts.join(" && ")})`;
    }
    case "And":
      return `(${e.args.map(go).join(" && ")})`;
    case "Or":
      return `(${e.args.map(go).join(" || ")})`;
    case "Not":
      return `(!${go(e.arg)})`;
    case "Tuple":
      // Output slots of a parametric body: one number per axis.
      return `[${e.items.map(go).join(", ")}]`;
    case "List": {
      // Rows of a matrix literal are flattened row-major.
      const leaves = e.items.flatMap((it) => (it.type === "List" ? it.items : [it]));
      if (leaves.some((l) => l.type === "List" || isArray(l.ty))) throw new CodegenError("nested list");
      return `[${leaves.map(go).join(", ")}]`;
    }
    case "Index": {
      const [m, n] = dims(e.base.ty);
      const b = go(e.base);
      if (e.base.ty.type === "Vector" && e.indices.length === 1) return `R.at(${b}, ${int(m)}, ${go(e.indices[0]!)})`;
      if (e.base.ty.type === "Matrix" && e.indices.length === 2) return `R.at2(${b}, ${int(m)}, ${int(n)}, ${go(e.indices[0]!)}, ${go(e.indices[1]!)})`;
      throw new CodegenError("bad index");
    }
    case "Split": {
      // Every line's condition is checked: more than one true at a point means the
      // boundary order broke at the current values (design.md "split"): undefined + warning.
      const lines = e.pieces.map((p) => `if (${go(p.cond)}) { v = ${go(p.value)}; n++; }`).join(" ");
      return `((s0) => { let v = NaN, n = 0; ${lines} if (n > 1) { R.overlap = true; return NaN; } return v; })(${go(e.subject)})`;
    }
    default:
      // Strings, matrices, addresses: never in an evaluated expression.
      throw new CodegenError(`unsupported node ${e.type}`);
  }
}

const CMP = { Lt: "<", Le: "<=", Gt: ">", Ge: ">=", Eq: "===" } as const;

/** Linear-algebra builtins, picked by the operand sizes from the type checker. */
function linalg(e: Extract<Expr, { type: "Call" }>, go: (c: Expr) => string): string {
  const a = e.args[0];
  if (!a) throw new CodegenError("missing argument");
  switch (e.callee.key) {
    case "matmul": {
      const b = e.args[1];
      if (!b || e.args.length !== 2) throw new CodegenError("matmul arity");
      const [m, k] = dims(a.ty), [k2, n] = dims(b.ty);
      if (k !== k2) throw new CodegenError("matmul size");
      return `R.matmul(${go(a)}, ${go(b)}, ${int(m)}, ${int(k)}, ${int(n)})`;
    }
    case "transpose": {
      const [m, n] = dims(a.ty);
      return `R.transpose(${go(a)}, ${int(m)}, ${int(n)})`;
    }
    case "inv":
    case "det":
    case "tr":
    case "pow": {
      const [m, n] = dims(a.ty);
      if (m !== n) throw new CodegenError("not square");
      if (e.callee.key === "pow") {
        const p = e.args[1];
        if (!p || e.args.length !== 2) throw new CodegenError("pow arity");
        return `R.mpow(${go(a)}, ${int(n)}, ${go(p)})`;
      }
      return `R.${e.callee.key}(${go(a)}, ${int(n)})`;
    }
    default:
      throw new CodegenError("unknown builtin");
  }
}

/** Compiles a constant-free scalar expression (view bounds). */
export function compileConst(e: Expr, R: Runtime): () => number {
  const src = `"use strict";\nreturn () => ${expr(e, { constIndex: new Map(), fnIndex: new Map() })};`;
  return new Function("R", "F", src)(R, []) as () => number;
}

/** `lo < hi` over the constants (split boundary order); throws if it needs more than constants. */
export function compileOrder(lo: Expr, hi: Expr, constIndex: Map<string, number>, R: Runtime): (C: Float64Array) => boolean {
  const env: CodegenEnv = { constIndex, fnIndex: new Map(), axes: null, inputs: [] };
  const src = `"use strict";\nreturn (C) => ${expr(lo, env)} < ${expr(hi, env)};`;
  return new Function("R", "F", src)(R, []) as (C: Float64Array) => boolean;
}

/** Compiles a resolved inequality region (highlight range slot, clip). */
export function compileRegion(e: Expr, env: CodegenEnv, R: Runtime, F: Scalar[]): Region {
  const src = `"use strict";\nreturn (x, y, C) => ${expr(e, env)};`;
  return new Function("R", "F", src)(R, F) as Region;
}

/**
 * Compiles one function. `F` is the shared table of scalar functions, filled by the caller;
 * calls resolve through it at run time, so compile order does not matter.
 */
export function compile(spec: FnSpec, env: CodegenEnv, R: Runtime, F: Scalar[], discreteCallee: boolean): Compiled {
  const none = () => NaN;
  const base = { key: spec.key, version: spec.version, kind: spec.kind, scalar: none as Scalar, batch: (() => {}) as Batch, table: none as Scalar, vecs: [] as Vec[], discrete: false };
  const make = (src: string) => new Function("R", "F", `"use strict";\nreturn ${src};`)(R, F);
  switch (spec.kind) {
    case "Matrix": {
      const ps = spec.inputs.map((_, i) => `x${i}`);
      if (ps.length === 0) throw new CodegenError("no inputs");
      const e = expr(spec.body, { ...env, axes: null, inputs: spec.inputs });
      return { ...base, table: make(`(${ps.join(", ")}, C) => ${e}`) as Scalar };
    }
    case "Value":
      return { ...base, table: make(`(C) => ${expr(spec.body, { ...env, axes: null, inputs: [] })}`) as Scalar };
    case "Span":
    case "Arrow": {
      if (spec.body.type !== "Call") throw new CodegenError("span body");
      const vecs = spec.body.args.map((a) => make(`(C) => ${expr(a, { ...env, axes: null, inputs: [] })}`) as Vec);
      return { ...base, vecs };
    }
    case "Feature":
      // Found by the worker (`features.ts`); the table entry is set there.
      return base;
    case "Calc":
      // Not evaluated: its steps are (`verify.ts`).
      return base;
    case "Parametric":
    case "Polar":
    case "Equation":
    case "Curve":
      break;
  }
  if (spec.family) {
    // One set of members per integer k: a factory with k bound to `k0`.
    const benv = { ...env, axes: null, inputs: [] };
    const at = make(`(k0) => (${drawnSource(spec, env, [spec.family.index])})`) as (k: number) => DrawnMembers;
    const krange = make(`(C) => [${expr(spec.family.lo, benv)}, ${expr(spec.family.hi, benv)}]`) as (C: Float64Array) => [number, number];
    return { ...base, family: { at, krange, loStrict: spec.family.lo_strict, hiStrict: spec.family.hi_strict } };
  }
  const m = make(drawnSource(spec, env, [])) as DrawnMembers;
  switch (spec.kind) {
    case "Parametric":
      return { ...base, ...m, table: m.param as unknown as Scalar };
    case "Polar":
      // Called as a number (r) from other functions.
      return { ...base, ...m, table: m.scalar! };
    case "Equation":
      return { ...base, ...m };
    default:
      return { ...base, ...m, table: m.scalar!, discrete: discreteCallee || boundUsesInput(spec.body) };
  }
}

/** What a drawn definition evaluates (per k for `family`). */
export interface DrawnMembers {
  scalar?: Scalar;
  batch?: Batch;
  param?: Param;
  trange?: (C: Float64Array) => [number, number];
  implicit?: Implicit;
}

/** Object-literal source of a drawn definition's members; `idx` = the family index (→ `k0`). */
function drawnSource(spec: FnSpec, env: CodegenEnv, idx: string[]): string {
  switch (spec.kind) {
    case "Parametric":
    case "Polar": {
      if (spec.inputs.length !== 1) throw new CodegenError("one parameter");
      const body = expr(spec.body, { ...env, axes: null, inputs: spec.inputs }, idx);
      const benv = { ...env, axes: null, inputs: [] };
      const trange = spec.range ? `(C) => [${expr(spec.range.lo, benv, idx)}, ${expr(spec.range.hi, benv, idx)}]` : `() => [0, 2 * Math.PI]`;
      if (spec.kind === "Parametric") return `{ param: (x0, C) => ${body}, trange: ${trange} }`;
      return `{ scalar: (x0, C) => ${body}, param: (x0, C) => { const r = ${body}; return [r * Math.cos(x0), r * Math.sin(x0)]; }, trange: ${trange} }`;
    }
    case "Equation": {
      if (spec.inputs.length !== 2 || spec.body.type !== "Compare" || spec.body.rest.length !== 1) throw new CodegenError("equation shape");
      const env2 = { ...env, axes: null, inputs: spec.inputs };
      return `{ implicit: (x0, x1, C) => (${expr(spec.body.first, env2, idx)}) - (${expr(spec.body.rest[0]!.rhs, env2, idx)}) }`;
    }
    case "Curve": {
      const e = expr(spec.body, { ...env, axes: null }, idx);
      return `{ scalar(x, C) { return ${e}; }, batch(xs, out, C) { const n = xs.length; for (let i = 0; i < n; i++) { const x = xs[i]; out[i] = ${e}; } } }`;
    }
    default:
      throw new CodegenError("not a drawn curve");
  }
}
