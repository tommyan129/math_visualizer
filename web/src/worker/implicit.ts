// Implicit curves g(x, y) = 0 (design.md "음함수: marching squares + 곡선 근처만 세분화").
// 1. Coarse grid over the computed area; cells where g changes sign, and their
//    neighbours, are refined into a finer grid.
// 2. Marching squares on the fine cells; each crossing is placed by bisection along
//    the cell edge. A "crossing" whose |g| does not shrink under bisection is a pole
//    (1/x = y changes sign across x = 0) and is dropped.
// 3. Segments are stitched into polylines through the edges they share.

import type { Implicit } from "./codegen";
import type { Polyline, Quality, SampleRange } from "./sample";

const TUNING: Record<Quality, { cellPx: number; sub: number }> = {
  coarse: { cellPx: 12, sub: 3 },
  fine: { cellPx: 8, sub: 4 },
};
/** Most coarse cells; the cell grows beyond this. */
const MAX_CELLS = 200_000;
/** Most fine evaluations before the result is marked truncated. */
const MAX_FINE = 2_000_000;
const BISECT_STEPS = 30;

export function sampleImplicit(g: Implicit, C: Float64Array, r: SampleRange, quality: Quality): Polyline {
  const tu = TUNING[quality];
  const wPx = (r.x1 - r.x0) * r.sx, hPx = (r.y1 - r.y0) * r.sy;
  let cell = tu.cellPx;
  while ((wPx / cell) * (hPx / cell) > MAX_CELLS) cell *= 1.25;
  const W = Math.max(1, Math.ceil(wPx / cell)), H = Math.max(1, Math.ceil(hPx / cell));
  const sub = tu.sub;
  const FW = W * sub, FH = H * sub;
  const fx = (I: number) => r.x0 + ((r.x1 - r.x0) * I) / FW;
  const fy = (J: number) => r.y0 + ((r.y1 - r.y0) * J) / FH;

  // Coarse values on fine-grid coordinates (every `sub`-th vertex).
  const vals = new Map<number, number>();
  const key = (I: number, J: number) => J * (FW + 1) + I;
  let evals = 0;
  const v = (I: number, J: number) => {
    const k = key(I, J);
    let x = vals.get(k);
    if (x === undefined) {
      x = g(fx(I), fy(J), C);
      vals.set(k, x);
      evals++;
    }
    return x;
  };
  const marked = new Uint8Array(W * H);
  for (let j = 0; j < H; j++) {
    for (let i = 0; i < W; i++) {
      const a = v(i * sub, j * sub), b = v((i + 1) * sub, j * sub), c = v(i * sub, (j + 1) * sub), d = v((i + 1) * sub, (j + 1) * sub);
      if (![a, b, c, d].every(Number.isFinite)) {
        // Partly undefined (sqrt edge, pole): the curve may still pass; look closer.
        if ([a, b, c, d].some(Number.isFinite)) marked[j * W + i] = 1;
        continue;
      }
      const pos = (a >= 0 ? 1 : 0) + (b >= 0 ? 1 : 0) + (c >= 0 ? 1 : 0) + (d >= 0 ? 1 : 0);
      if (pos !== 0 && pos !== 4) marked[j * W + i] = 1;
    }
  }
  // Neighbours too: a curve can dip into a cell between corners of equal sign.
  const refine = new Uint8Array(W * H);
  for (let j = 0; j < H; j++) {
    for (let i = 0; i < W; i++) {
      if (!marked[j * W + i]) continue;
      for (let dj = -1; dj <= 1; dj++) {
        for (let di = -1; di <= 1; di++) {
          const ii = i + di, jj = j + dj;
          if (ii >= 0 && jj >= 0 && ii < W && jj < H) refine[jj * W + ii] = 1;
        }
      }
    }
  }

  // Crossing point on a fine edge (id: 2·vertex + 0 horizontal / 1 vertical); null = pole.
  const points = new Map<number, [number, number] | null>();
  const crossing = (I: number, J: number, vertical: boolean, va: number, vb: number): number => {
    const id = key(I, J) * 2 + (vertical ? 1 : 0);
    if (!points.has(id)) {
      let a = 0, b = 1, ga = va, gb = vb;
      const x0 = fx(I), y0 = fy(J);
      const x1 = vertical ? x0 : fx(I + 1), y1 = vertical ? fy(J + 1) : y0;
      const at = (s: number) => g(x0 + (x1 - x0) * s, y0 + (y1 - y0) * s, C);
      for (let k = 0; k < BISECT_STEPS; k++) {
        const m = (a + b) / 2, gm = at(m);
        if (!Number.isFinite(gm)) break;
        if ((gm >= 0) === (ga >= 0)) {
          a = m;
          ga = gm;
        } else {
          b = m;
          gb = gm;
        }
      }
      const shrunk = Math.min(Math.abs(ga), Math.abs(gb)) <= 1e-6 * Math.max(1, Math.abs(va), Math.abs(vb)) || Math.min(Math.abs(ga), Math.abs(gb)) < 0.5 * Math.min(Math.abs(va), Math.abs(vb));
      const s = Math.abs(ga) + Math.abs(gb) > 0 ? a + (b - a) * (Math.abs(ga) / (Math.abs(ga) + Math.abs(gb))) : (a + b) / 2;
      points.set(id, shrunk && Number.isFinite(ga) && Number.isFinite(gb) ? [x0 + (x1 - x0) * s, y0 + (y1 - y0) * s] : null);
    }
    return id;
  };

  // Marching squares on refined cells.
  const segs: [number, number][] = [];
  let truncated = false;
  outer: for (let j = 0; j < H; j++) {
    for (let i = 0; i < W; i++) {
      if (!refine[j * W + i]) continue;
      for (let sj = 0; sj < sub; sj++) {
        for (let si = 0; si < sub; si++) {
          if (evals > MAX_FINE) {
            truncated = true;
            break outer;
          }
          const I = i * sub + si, J = j * sub + sj;
          const a = v(I, J), b = v(I + 1, J), c = v(I, J + 1), d = v(I + 1, J + 1);
          if (![a, b, c, d].every(Number.isFinite)) continue;
          const sa = a >= 0, sb = b >= 0, sc = c >= 0, sd = d >= 0;
          // Edges: bottom (a-b), right (b-d), top (c-d), left (a-c).
          const e: number[] = [];
          const bottom = () => crossing(I, J, false, a, b);
          const right = () => crossing(I + 1, J, true, b, d);
          const top = () => crossing(I, J + 1, false, c, d);
          const left = () => crossing(I, J, true, a, c);
          if (sa !== sb) e.push(bottom());
          if (sb !== sd) e.push(right());
          if (sc !== sd) e.push(top());
          if (sa !== sc) e.push(left());
          if (e.length === 2) segs.push([e[0]!, e[1]!]);
          else if (e.length === 4) {
            // Saddle: the centre's sign decides which corners connect.
            const m = g((fx(I) + fx(I + 1)) / 2, (fy(J) + fy(J + 1)) / 2, C);
            evals++;
            const [eb, er, et, el] = e as [number, number, number, number];
            if ((m >= 0) === sa) {
              segs.push([eb, er], [et, el]);
            } else {
              segs.push([eb, el], [et, er]);
            }
          }
        }
      }
    }
  }

  // Stitch through shared edges; drop segments touching a pole.
  const valid = segs.filter(([p, q]) => points.get(p) && points.get(q));
  const byEdge = new Map<number, number[]>();
  valid.forEach(([p, q], s) => {
    for (const e of [p, q]) {
      const l = byEdge.get(e);
      if (l) l.push(s);
      else byEdge.set(e, [s]);
    }
  });
  const used = new Uint8Array(valid.length);
  const xs: number[] = [], ys: number[] = [], offsets: number[] = [];
  const push = (e: number) => {
    const p = points.get(e)!;
    xs.push(p[0]);
    ys.push(p[1]);
  };
  const walk = (start: number, from: number): number[] => {
    // Edges from `from` along unused segments, starting with segment `start`.
    const chain: number[] = [];
    let s = start, e = from;
    while (s >= 0 && !used[s]) {
      used[s] = 1;
      const [p, q] = valid[s]!;
      e = p === e ? q : p;
      chain.push(e);
      s = (byEdge.get(e) ?? []).find((t) => !used[t]) ?? -1;
    }
    return chain;
  };
  for (let s = 0; s < valid.length; s++) {
    if (used[s]) continue;
    const [p, q] = valid[s]!;
    used[s] = 1;
    // Extend both ways from this segment.
    const fwd = walk((byEdge.get(q) ?? []).find((t) => !used[t]) ?? -1, q);
    const back = walk((byEdge.get(p) ?? []).find((t) => !used[t]) ?? -1, p);
    offsets.push(xs.length);
    for (let k = back.length - 1; k >= 0; k--) push(back[k]!);
    push(p);
    push(q);
    for (const e of fwd) push(e);
  }
  offsets.push(xs.length);
  return { xs: Float64Array.from(xs), ys: Float64Array.from(ys), offsets, truncated };
}
