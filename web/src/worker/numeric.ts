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
  // Vectors and matrices (stage 4): row-major number arrays, sizes from the static types.
  madd(a: number[], b: number[]): number[];
  msub(a: number[], b: number[]): number[];
  mscale(s: number, a: number[]): number[];
  /** (m×k) · (k×n). */
  matmul(a: number[], b: number[], m: number, k: number, n: number): number[];
  transpose(a: number[], m: number, n: number): number[];
  inv(a: number[], n: number): number[];
  det(a: number[], n: number): number;
  tr(a: number[], n: number): number;
  /** Integer power of an n×n matrix (negative: of the inverse). */
  mpow(a: number[], n: number, p: number): number[];
  /** 1-based element; NaN outside or for a non-integer index. */
  at(a: number[], len: number, i: number): number;
  at2(a: number[], m: number, n: number, i: number, j: number): number;
}

const nanArray = (len: number) => new Array<number>(len).fill(NaN);

/** Gauss-Jordan with partial pivoting. Singular (pivot ~ 0 relative to the scale) → NaN. */
function inverse(a: number[], n: number): number[] {
  const w = 2 * n;
  const m = new Array<number>(n * w);
  let scale = 0;
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n; j++) {
      const v = a[i * n + j]!;
      m[i * w + j] = v;
      scale = Math.max(scale, Math.abs(v));
      m[i * w + n + j] = i === j ? 1 : 0;
    }
  }
  if (!Number.isFinite(scale)) return nanArray(n * n);
  for (let c = 0; c < n; c++) {
    let p = c;
    for (let r = c + 1; r < n; r++) if (Math.abs(m[r * w + c]!) > Math.abs(m[p * w + c]!)) p = r;
    const pv = m[p * w + c]!;
    if (!(Math.abs(pv) > 1e-12 * scale)) return nanArray(n * n);
    if (p !== c) {
      for (let j = 0; j < w; j++) {
        const t = m[c * w + j]!;
        m[c * w + j] = m[p * w + j]!;
        m[p * w + j] = t;
      }
    }
    for (let j = 0; j < w; j++) m[c * w + j]! /= pv;
    for (let r = 0; r < n; r++) {
      if (r === c) continue;
      const f = m[r * w + c]!;
      if (f === 0) continue;
      for (let j = 0; j < w; j++) m[r * w + j]! -= f * m[c * w + j]!;
    }
  }
  const out = new Array<number>(n * n);
  for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) out[i * n + j] = m[i * w + n + j]!;
  return out;
}

/** LU with partial pivoting. */
function determinant(a: number[], n: number): number {
  const m = a.slice();
  let d = 1;
  for (let c = 0; c < n; c++) {
    let p = c;
    for (let r = c + 1; r < n; r++) if (Math.abs(m[r * n + c]!) > Math.abs(m[p * n + c]!)) p = r;
    const pv = m[p * n + c]!;
    if (pv === 0) return 0;
    if (Number.isNaN(pv)) return NaN;
    if (p !== c) {
      for (let j = 0; j < n; j++) {
        const t = m[c * n + j]!;
        m[c * n + j] = m[p * n + j]!;
        m[p * n + j] = t;
      }
      d = -d;
    }
    d *= pv;
    for (let r = c + 1; r < n; r++) {
      const f = m[r * n + c]! / pv;
      for (let j = c; j < n; j++) m[r * n + j]! -= f * m[c * n + j]!;
    }
  }
  return d;
}

function matmul(a: number[], b: number[], m: number, k: number, n: number): number[] {
  const out = new Array<number>(m * n);
  for (let i = 0; i < m; i++) {
    for (let j = 0; j < n; j++) {
      let s = 0;
      for (let t = 0; t < k; t++) s += a[i * k + t]! * b[t * n + j]!;
      out[i * n + j] = s;
    }
  }
  return out;
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
    madd: (a, b) => a.map((v, i) => v + b[i]!),
    msub: (a, b) => a.map((v, i) => v - b[i]!),
    mscale: (s, a) => a.map((v) => s * v),
    matmul,
    transpose(a, m, n) {
      const out = new Array<number>(m * n);
      for (let i = 0; i < m; i++) for (let j = 0; j < n; j++) out[j * m + i] = a[i * n + j]!;
      return out;
    },
    inv: inverse,
    det: determinant,
    tr(a, n) {
      let s = 0;
      for (let i = 0; i < n; i++) s += a[i * n + i]!;
      return s;
    },
    mpow(a, n, p) {
      if (!Number.isInteger(p)) return nanArray(n * n);
      let base = p < 0 ? inverse(a, n) : a;
      let e = Math.abs(p);
      let out = new Array<number>(n * n).fill(0);
      for (let i = 0; i < n; i++) out[i * n + i] = 1;
      while (e > 0) {
        if (e % 2 === 1) out = matmul(out, base, n, n, n);
        e = Math.floor(e / 2);
        if (e > 0) base = matmul(base, base, n, n, n);
      }
      return out;
    },
    at(a, len, i) {
      return Number.isInteger(i) && i >= 1 && i <= len ? a[i - 1]! : NaN;
    },
    at2(a, m, n, i, j) {
      return Number.isInteger(i) && Number.isInteger(j) && i >= 1 && i <= m && j >= 1 && j <= n ? a[(i - 1) * n + (j - 1)]! : NaN;
    },
  };
  rt.sum = bigOp(rt, 0, (a, v) => a + v);
  rt.prod = bigOp(rt, 1, (a, v) => a * v);
  return rt;
}
