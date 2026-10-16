// Adaptive sampling of parametric and polar curves (design.md "샘플링"): the same
// screen-space rules as y = f(x), over the parameter t instead of x.
// - start uniform in t, subdivide where the screen direction turns or a segment is long,
// - approach domain edges (finite next to undefined),
// - a long screen jump is bisected; if it does not shrink, the line breaks there.

import type { Param } from "./codegen";
import { MAX_POINTS, type Polyline, type Quality, type SampleRange } from "./sample";

interface Tuning {
  n0: number;
  maxDepth: number;
  angle: number;
  minPx: number;
  /** A segment longer than this on screen is always subdivided. */
  maxPx: number;
  jumpPx: number;
}

const TUNING: Record<Quality, Tuning> = {
  coarse: { n0: 200, maxDepth: 4, angle: 0.2, minPx: 3, maxPx: 40, jumpPx: 60 },
  fine: { n0: 600, maxDepth: 9, angle: 0.05, minPx: 0.5, maxPx: 12, jumpPx: 30 },
};

const BISECT_STEPS = 40;
const CONTINUOUS_RATIO = 0.01;

/** Does the screen gap between t = a and t = b survive bisection (a jump, not a fast stretch)? */
function isJump(f: Param, C: Float64Array, a: number, pa: number[], b: number, pb: number[], r: SampleRange): boolean {
  const d = (p: number[], q: number[]) => Math.hypot((q[0]! - p[0]!) * r.sx, (q[1]! - p[1]!) * r.sy);
  const initial = d(pa, pb);
  for (let i = 0; i < BISECT_STEPS; i++) {
    const m = (a + b) / 2;
    if (m === a || m === b) break;
    const pm = f(m, C);
    if (!Number.isFinite(pm[0]!) || !Number.isFinite(pm[1]!)) return true;
    if (d(pa, pm) >= d(pm, pb)) {
      b = m;
      pb = pm;
    } else {
      a = m;
      pa = pm;
    }
    if (d(pa, pb) < initial * CONTINUOUS_RATIO) return false;
  }
  return d(pa, pb) >= initial * CONTINUOUS_RATIO;
}

export function sampleParametric(f: Param, C: Float64Array, t0: number, t1: number, r: SampleRange, quality: Quality): Polyline {
  const empty: Polyline = { xs: new Float64Array(0), ys: new Float64Array(0), offsets: [0], truncated: false };
  if (!Number.isFinite(t0) || !Number.isFinite(t1) || t1 < t0) return empty;
  const tu = TUNING[quality];
  const n0 = t1 === t0 ? 1 : tu.n0;
  let ts: number[] = [];
  for (let i = 0; i <= n0; i++) ts.push(n0 === 0 ? t0 : t0 + ((t1 - t0) * i) / n0);
  const at = (t: number) => {
    const p = f(t, C);
    return [p[0] ?? NaN, p[1] ?? NaN];
  };
  let ps: number[][] = ts.map(at);
  const ok = (p: number[]) => Number.isFinite(p[0]!) && Number.isFinite(p[1]!);
  // Segments wholly on one side outside the computed area are not refined.
  const out = (p: number[], q: number[]) =>
    (p[0]! < r.x0 && q[0]! < r.x0) || (p[0]! > r.x1 && q[0]! > r.x1) || (p[1]! < r.y0 && q[1]! < r.y0) || (p[1]! > r.y1 && q[1]! > r.y1);
  const dir = (p: number[], q: number[]) => Math.atan2((q[1]! - p[1]!) * r.sy, (q[0]! - p[0]!) * r.sx);
  const turn = (a: number, b: number) => {
    let d = Math.abs(a - b);
    if (d > Math.PI) d = 2 * Math.PI - d;
    return d;
  };
  let truncated = false;
  for (let depth = 0; depth < tu.maxDepth; depth++) {
    const mids: number[] = [];
    const after: number[] = [];
    for (let i = 0; i + 1 < ts.length; i++) {
      const a = ps[i]!, b = ps[i + 1]!;
      let need = false;
      if (ok(a) !== ok(b)) need = true;
      else if (ok(a) && ok(b) && !out(a, b)) {
        const len = Math.hypot((b[0]! - a[0]!) * r.sx, (b[1]! - a[1]!) * r.sy);
        if (len > tu.maxPx) need = true;
        else if (len > tu.minPx) {
          const here = dir(a, b);
          const prev = i > 0 && ok(ps[i - 1]!) ? dir(ps[i - 1]!, a) : null;
          const next = i + 2 < ps.length && ok(ps[i + 2]!) ? dir(b, ps[i + 2]!) : null;
          if ((prev !== null && turn(here, prev) > tu.angle) || (next !== null && turn(next, here) > tu.angle)) need = true;
        }
      }
      if (need) {
        mids.push((ts[i]! + ts[i + 1]!) / 2);
        after.push(i);
      }
    }
    if (mids.length === 0) break;
    if (ts.length + mids.length > MAX_POINTS) {
      truncated = true;
      break;
    }
    const nt: number[] = [], np: number[][] = [];
    let k = 0;
    for (let i = 0; i < ts.length; i++) {
      nt.push(ts[i]!);
      np.push(ps[i]!);
      if (k < after.length && after[k] === i) {
        nt.push(mids[k]!);
        np.push(at(mids[k]!));
        k++;
      }
    }
    ts = nt;
    ps = np;
  }

  // Pieces: break at undefined points and at confirmed jumps.
  const xs: number[] = [], ys: number[] = [], offsets: number[] = [];
  let open = false;
  for (let i = 0; i < ps.length; i++) {
    const p = ps[i]!;
    if (!ok(p)) {
      open = false;
      continue;
    }
    if (open) {
      const q = ps[i - 1]!;
      const len = Math.hypot((p[0]! - q[0]!) * r.sx, (p[1]! - q[1]!) * r.sy);
      if (len > tu.jumpPx && !out(p, q) && isJump(f, C, ts[i - 1]!, q, ts[i]!, p, r)) open = false;
    }
    if (!open) {
      offsets.push(xs.length);
      open = true;
    }
    xs.push(p[0]!);
    ys.push(p[1]!);
  }
  offsets.push(xs.length);
  // Drop single-point pieces (nothing to draw as a line).
  const fx: number[] = [], fy: number[] = [], fo: number[] = [];
  for (let q = 0; q + 1 < offsets.length; q++) {
    const s = offsets[q]!, e = offsets[q + 1]!;
    if (e - s < 2) continue;
    fo.push(fx.length);
    for (let i = s; i < e; i++) {
      fx.push(xs[i]!);
      fy.push(ys[i]!);
    }
  }
  fo.push(fx.length);
  return { xs: Float64Array.from(fx), ys: Float64Array.from(fy), offsets: fo, truncated };
}
