// Adaptive sampling of y = f(x) (design.md "샘플링"):
// - recursive subdivision where the screen-space angle changes,
// - discontinuity detection: a big jump is re-checked by bisecting toward it;
//   if the jump survives, the line is broken instead of drawing a vertical segment.
// Evaluation is batched (array loops) per refinement round.

import type { Batch, Scalar } from "./codegen";

export type Quality = "coarse" | "fine";

export interface SampleRange {
  /** World x range to cover (visible area plus margin). */
  x0: number;
  x1: number;
  /** Visible y range; values far outside are clipped. */
  y0: number;
  y1: number;
  /** Screen pixels per world unit. */
  sx: number;
  sy: number;
}

export interface Polyline {
  xs: Float64Array;
  ys: Float64Array;
  /** Start index of each piece, plus the final end. */
  offsets: number[];
  truncated: boolean;
}

interface Tuning {
  /** Initial spacing in pixels. */
  step: number;
  maxDepth: number;
  /** Subdivide when the direction changes by more than this (radians). */
  angle: number;
  /** Below this screen length a segment is never subdivided for angle. */
  minPx: number;
  /** Screen jump that triggers the discontinuity re-check. */
  jumpPx: number;
}

const TUNING: Record<Quality, Tuning> = {
  coarse: { step: 6, maxDepth: 3, angle: 0.2, minPx: 3, jumpPx: 60 },
  fine: { step: 2, maxDepth: 8, angle: 0.05, minPx: 0.5, jumpPx: 30 },
};

/** Per-function sample cap (scene budget is checked by the caller). */
export const MAX_POINTS = 200_000;

const BISECT_STEPS = 40;
/** After bisection, a jump that kept at least this share of its size is a discontinuity. */
const CONTINUOUS_RATIO = 0.01;

function evalBatch(batch: Batch, xs: Float64Array, C: Float64Array): Float64Array {
  const out = new Float64Array(xs.length);
  batch(xs, out, C);
  return out;
}

/**
 * Is the jump between (a, ya) and (b, yb) a real discontinuity? Bisect toward the
 * larger half-jump. For a continuous function the jump shrinks with the interval;
 * at a jump or a pole it does not. The test is relative, so it does not depend on
 * the zoom or on how large the values are.
 */
export function isDiscontinuity(f: Scalar, C: Float64Array, a: number, ya: number, b: number, yb: number): boolean {
  const initial = Math.abs(yb - ya);
  for (let i = 0; i < BISECT_STEPS; i++) {
    const m = (a + b) / 2;
    if (m === a || m === b) break;
    const ym = f(m, C);
    if (!Number.isFinite(ym)) return true;
    if (Math.abs(ym - ya) >= Math.abs(yb - ym)) {
      b = m;
      yb = ym;
    } else {
      a = m;
      ya = ym;
    }
    if (Math.abs(yb - ya) < initial * CONTINUOUS_RATIO) return false;
  }
  return Math.abs(yb - ya) >= initial * CONTINUOUS_RATIO;
}

/** A jump smaller than this on screen is never worth breaking the line for. */
const MIN_JUMP_PX = 1.5;
/** A segment this many times steeper than both neighbours is checked for a jump. */
const STEEP_RATIO = 8;

/**
 * Should the segment (i-1, i) be checked for a discontinuity? Either it jumps a lot
 * on screen, or it is a small jump that stands out against its neighbours' slopes
 * (zoomed out, a real jump can be only a few px tall).
 */
function suspectJump(xs: number[], ys: number[], i: number, r: SampleRange, t: Tuning): boolean {
  const dyPx = Math.abs(ys[i]! - ys[i - 1]!) * r.sy;
  if (dyPx > t.jumpPx) return true;
  if (dyPx < MIN_JUMP_PX) return false;
  const slope = (j: number) => {
    if (j < 1 || j >= xs.length) return 0;
    const s = Math.abs(ys[j]! - ys[j - 1]!) / (xs[j]! - xs[j - 1]!);
    return Number.isFinite(s) ? s : 0;
  };
  return slope(i) > STEEP_RATIO * Math.max(slope(i - 1), slope(i + 1));
}

export function sampleCurve(batch: Batch, scalar: Scalar, C: Float64Array, r: SampleRange, quality: Quality): Polyline {
  const t = TUNING[quality];
  const widthPx = (r.x1 - r.x0) * r.sx;
  const n0 = Math.max(2, Math.min(Math.ceil(widthPx / t.step) + 1, 20_000));
  let xs: number[] = new Array(n0);
  for (let i = 0; i < n0; i++) xs[i] = r.x0 + ((r.x1 - r.x0) * i) / (n0 - 1);
  let ys: number[] = Array.from(evalBatch(batch, Float64Array.from(xs), C));
  let truncated = false;

  // Refinement rounds: collect midpoints of intervals that need it, evaluate them together.
  for (let depth = 0; depth < t.maxDepth; depth++) {
    const mids: number[] = [];
    const at: number[] = [];
    for (let i = 0; i + 1 < xs.length; i++) {
      const xa = xs[i]!, xb = xs[i + 1]!, ya = ys[i]!, yb = ys[i + 1]!;
      const fa = Number.isFinite(ya), fb = Number.isFinite(yb);
      let need = false;
      // Entirely above or below the computed area: nothing visible to refine.
      const off = fa && fb && ((ya > r.y1 && yb > r.y1) || (ya < r.y0 && yb < r.y0));
      if (off) {
        need = false;
      } else if (fa !== fb) {
        // Domain edge: approach it.
        need = true;
      } else if (fa && fb) {
        const dx = (xb - xa) * r.sx, dy = (yb - ya) * r.sy;
        const len = Math.hypot(dx, dy);
        if (len > t.minPx) {
          // Direction change against neighbours.
          const prev = i > 0 && Number.isFinite(ys[i - 1]!) ? Math.atan2((ya - ys[i - 1]!) * r.sy, (xa - xs[i - 1]!) * r.sx) : null;
          const next = i + 2 < xs.length && Number.isFinite(ys[i + 2]!) ? Math.atan2((ys[i + 2]! - yb) * r.sy, (xs[i + 2]! - xb) * r.sx) : null;
          const here = Math.atan2(dy, dx);
          if ((prev !== null && Math.abs(here - prev) > t.angle) || (next !== null && Math.abs(next - here) > t.angle)) need = true;
          if (Math.abs(dy) > t.jumpPx) need = true;
        }
      }
      if (need) {
        mids.push((xa + xb) / 2);
        at.push(i);
      }
    }
    if (mids.length === 0) break;
    if (xs.length + mids.length > MAX_POINTS) {
      truncated = true;
      break;
    }
    const my = evalBatch(batch, Float64Array.from(mids), C);
    const nx: number[] = [], ny: number[] = [];
    let k = 0;
    for (let i = 0; i < xs.length; i++) {
      nx.push(xs[i]!);
      ny.push(ys[i]!);
      if (k < at.length && at[k] === i) {
        nx.push(mids[k]!);
        ny.push(my[k]!);
        k++;
      }
    }
    xs = nx;
    ys = ny;
  }

  // Build pieces: break at non-finite values and at confirmed discontinuities; clip divergence.
  const span = (r.y1 - r.y0) * 10;
  const lo = r.y0 - span, hi = r.y1 + span;
  const outX: number[] = [], outY: number[] = [];
  const offsets: number[] = [];
  let open = false;
  for (let i = 0; i < xs.length; i++) {
    const y = ys[i]!;
    if (!Number.isFinite(y)) {
      open = false;
      continue;
    }
    if (open) {
      const py = ys[i - 1]!;
      const offSide = (py > r.y1 && y > r.y1) || (py < r.y0 && y < r.y0);
      if (!offSide && suspectJump(xs, ys, i, r, t) && isDiscontinuity(scalar, C, xs[i - 1]!, py, xs[i]!, y)) open = false;
    }
    if (!open) {
      offsets.push(outX.length);
      open = true;
    }
    outX.push(xs[i]!);
    outY.push(Math.min(hi, Math.max(lo, y)));
  }
  offsets.push(outX.length);
  // Drop single-point pieces (nothing to draw as a line).
  const fx: number[] = [], fy: number[] = [], fo: number[] = [];
  for (let p = 0; p + 1 < offsets.length; p++) {
    const s = offsets[p]!, e = offsets[p + 1]!;
    if (e - s < 2) continue;
    fo.push(fx.length);
    for (let i = s; i < e; i++) {
      fx.push(outX[i]!);
      fy.push(outY[i]!);
    }
  }
  fo.push(fx.length);
  return { xs: Float64Array.from(fx), ys: Float64Array.from(fy), offsets: fo, truncated };
}

/** Functions defined only at integer x: one point per integer in range. */
export const MAX_DISCRETE = 20_000;

export function sampleDiscrete(batch: Batch, C: Float64Array, r: SampleRange): { xs: Float64Array; ys: Float64Array; truncated: boolean } {
  let a = Math.ceil(r.x0), b = Math.floor(r.x1);
  let truncated = false;
  if (b - a + 1 > MAX_DISCRETE) {
    const mid = Math.round((r.x0 + r.x1) / 2);
    a = mid - MAX_DISCRETE / 2;
    b = mid + MAX_DISCRETE / 2 - 1;
    truncated = true;
  }
  if (b < a) return { xs: new Float64Array(0), ys: new Float64Array(0), truncated };
  const xs = new Float64Array(b - a + 1);
  for (let i = 0; i < xs.length; i++) xs[i] = a + i;
  const ys = evalBatch(batch, xs, C);
  const fx: number[] = [], fy: number[] = [];
  for (let i = 0; i < xs.length; i++) {
    if (Number.isFinite(ys[i]!)) {
      fx.push(xs[i]!);
      fy.push(ys[i]!);
    }
  }
  return { xs: Float64Array.from(fx), ys: Float64Array.from(fy), truncated };
}
