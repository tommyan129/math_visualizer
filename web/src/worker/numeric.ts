// Numeric semantics (design.md "수치 의미 명세"). Real mode: undefined → NaN.
// Generated code calls only these helpers and Math through this object.

/** Maximum terms in one sum/prod evaluation; beyond it the value is NaN and a warning is raised. */
export const TERM_CAP = 100_000;

export interface Runtime {
  PI: number;
  E: number;
  /** Set when a sum/prod hit TERM_CAP since the last reset. */
  capped: boolean;
  pow(b: number, e: number): number;
  powRat(b: number, p: number, q: number): number;
  fact(n: number): number;
  gamma(x: number): number;
  round(x: number): number;
  mod(a: number, m: number): number;
  deg(x: number): number;
  log(x: number, b: number): number;
  sum(lo: number, hi: number, loStrict: boolean, hiStrict: boolean, body: (k: number) => number): number;
  prod(lo: number, hi: number, loStrict: boolean, hiStrict: boolean, body: (k: number) => number): number;
  sin: (x: number) => number;
  cos: (x: number) => number;
  tan: (x: number) => number;
  asin: (x: number) => number;
  acos: (x: number) => number;
  atan: (x: number) => number;
  sinh: (x: number) => number;
  cosh: (x: number) => number;
  tanh: (x: number) => number;
  exp: (x: number) => number;
  ln: (x: number) => number;
  log10: (x: number) => number;
  log2: (x: number) => number;
  sqrt: (x: number) => number;
  abs: (x: number) => number;
  sign: (x: number) => number;
  floor: (x: number) => number;
  ceil: (x: number) => number;
}

// Lanczos approximation (g = 7, n = 9).
const LANCZOS = [
  0.99999999999980993, 676.5203681218851, -1259.1392167224028, 771.32342877765313, -176.61502916214059, 12.507343278686905,
  -0.13857109526572012, 9.9843695780195716e-6, 1.5056327351493116e-7,
];

function gamma(x: number): number {
  if (Number.isNaN(x)) return NaN;
  if (Number.isInteger(x) && x <= 0) return NaN;
  if (x < 0.5) return Math.PI / (Math.sin(Math.PI * x) * gamma(1 - x));
  const z = x - 1;
  let a = LANCZOS[0]!;
  const t = z + 7.5;
  for (let i = 1; i < 9; i++) a += LANCZOS[i]! / (z + i);
  return Math.sqrt(2 * Math.PI) * Math.pow(t, z + 0.5) * Math.exp(-t) * a;
}

function bigOp(
  rt: Runtime,
  unit: number,
  combine: (acc: number, v: number) => number,
): (lo: number, hi: number, ls: boolean, hs: boolean, body: (k: number) => number) => number {
  return (lo, hi, ls, hs, body) => {
    // The index range is defined only for integer bounds.
    if (!Number.isInteger(lo) || !Number.isInteger(hi)) return NaN;
    const start = ls ? lo + 1 : lo;
    const end = hs ? hi - 1 : hi;
    if (end < start) return unit;
    if (end - start + 1 > TERM_CAP) {
      rt.capped = true;
      return NaN;
    }
    let acc = unit;
    for (let k = start; k <= end; k++) acc = combine(acc, body(k));
    return acc;
  };
}

export function makeRuntime(): Runtime {
  const rt: Runtime = {
    PI: Math.PI,
    E: Math.E,
    capped: false,
    // 0^0 = 1 (Math.pow already does this).
    pow: Math.pow,
    // Negative base with a reduced rational exponent p/q: real root when q is odd.
    powRat(b, p, q) {
      if (b >= 0 || Number.isNaN(b)) return Math.pow(b, p / q);
      if (q % 2 === 0) return NaN;
      const r = Math.pow(-b, p / q);
      return p % 2 === 0 ? r : -r;
    },
    fact(n) {
      if (!Number.isInteger(n) || n < 0) return NaN;
      if (n > 170) return Infinity;
      let r = 1;
      for (let i = 2; i <= n; i++) r *= i;
      return r;
    },
    gamma,
    // Half away from zero (Rust rounding): -2.5 → -3.
    round(x) {
      return Math.sign(x) * Math.floor(Math.abs(x) + 0.5);
    },
    // Always 0 <= r < |m| (Rust rem_euclid).
    mod(a, m) {
      if (m === 0) return NaN;
      const r = a % m;
      return r < 0 ? r + Math.abs(m) : r;
    },
    deg(x) {
      return (x * Math.PI) / 180;
    },
    log(x, b) {
      return Math.log(x) / Math.log(b);
    },
    sum: () => NaN,
    prod: () => NaN,
    sin: Math.sin,
    cos: Math.cos,
    tan: Math.tan,
    asin: Math.asin,
    acos: Math.acos,
    atan: Math.atan,
    sinh: Math.sinh,
    cosh: Math.cosh,
    tanh: Math.tanh,
    exp: Math.exp,
    ln: Math.log,
    log10: Math.log10,
    log2: Math.log2,
    sqrt: Math.sqrt,
    abs: Math.abs,
    sign: Math.sign,
    floor: Math.floor,
    ceil: Math.ceil,
  };
  rt.sum = bigOp(rt, 0, (a, v) => a + v);
  rt.prod = bigOp(rt, 1, (a, v) => a * v);
  return rt;
}
