// span and arrow geometry (design.md "span", stage-4 decisions). Pure functions.
//
// Tolerance (temporary, revisit with the stage-5 numeric verification tolerance):
// two vectors are parallel when |v×w| <= 1e-9·|v|·|w|; a vector is zero when its
// norm is <= 1e-12 times the largest component among the span's vectors.

export const PARALLEL_TOL = 1e-9;
export const ZERO_TOL = 1e-12;

export interface SpanShape {
  /** 0 = origin, 1 = line, 2 = plane; null = a vector is not a finite number. */
  dim: 0 | 1 | 2 | null;
  /** Direction of the line (dim 1). */
  dir: [number, number] | null;
}

/** Shape of the span of 2D vectors at their current values. */
export function spanShape(vecs: number[][]): SpanShape {
  if (vecs.some((v) => v.length !== 2 || !v.every(Number.isFinite))) return { dim: null, dir: null };
  const scale = Math.max(0, ...vecs.flat().map(Math.abs));
  const nonzero = vecs.filter((v) => Math.hypot(v[0]!, v[1]!) > ZERO_TOL * scale);
  if (scale === 0 || nonzero.length === 0) return { dim: 0, dir: null };
  const d = nonzero[0]!;
  const nd = Math.hypot(d[0]!, d[1]!);
  for (const w of nonzero.slice(1)) {
    const cross = d[0]! * w[1]! - d[1]! * w[0]!;
    if (Math.abs(cross) > PARALLEL_TOL * nd * Math.hypot(w[0]!, w[1]!)) return { dim: 2, dir: null };
  }
  return { dim: 1, dir: [d[0]! / nd, d[1]! / nd] };
}

export interface Box {
  x0: number;
  x1: number;
  y0: number;
  y1: number;
}

/** The part of the line t·dir inside `box` (Liang-Barsky), or null when it misses. */
export function clipLine(dir: [number, number], box: Box): [number, number, number, number] | null {
  let lo = -Infinity, hi = Infinity;
  const [dx, dy] = dir;
  for (const [d, a, b] of [[dx, box.x0, box.x1], [dy, box.y0, box.y1]] as const) {
    if (d === 0) {
      if (0 < a || 0 > b) return null;
      continue;
    }
    const t1 = a / d, t2 = b / d;
    lo = Math.max(lo, Math.min(t1, t2));
    hi = Math.min(hi, Math.max(t1, t2));
  }
  if (!(hi > lo)) return null;
  return [lo * dx, lo * dy, hi * dx, hi * dy];
}
