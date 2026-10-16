// Highlight geometry (design.md "강조"):
// - function target: polygons between the curve and the axis (or the top / bottom of the
//   computed area for above / below), restricted to where the range holds. Range edges
//   are found by bisection so they do not snap to the sampling grid.
// - targetless region: a raster mask at screen resolution, one sample per pixel.

import type { Region, Scalar } from "./codegen";
import type { SampleRange } from "./sample";

export type FillRule = "Axis" | "Above" | "Below";

export interface FillGeometry {
  /** World x, y pairs: every ring, one after another. */
  xy: number[];
  /** Start of each ring plus the final end (in points). */
  rings: number[];
  /** The curve part of each ring (boundary line), as polyline pieces. */
  edge: number[];
  edgeOffsets: number[];
}

const BISECT = 40;

/**
 * Fill for y = f(x) over `range.x0..x1` with `n` uniform intervals. `cond` restricts x
 * (null = everywhere the function is defined).
 */
export function computeFill(f: Scalar, cond: Region | null, rule: FillRule, range: SampleRange, n: number, C: Float64Array): FillGeometry {
  const h = range.y1 - range.y0;
  const lo = range.y0 - h, hi = range.y1 + h;
  const clip = (y: number) => Math.min(hi, Math.max(lo, y));
  const ok = (x: number) => {
    const y = f(x, C);
    return Number.isFinite(y) && (!cond || cond(x, y, C));
  };
  const base = rule === "Axis" ? 0 : rule === "Above" ? hi : lo;
  const out: FillGeometry = { xy: [], rings: [], edge: [], edgeOffsets: [] };
  let run: number[] = [];
  const close = () => {
    if (run.length >= 4) {
      out.rings.push(out.xy.length / 2);
      out.edgeOffsets.push(out.edge.length / 2);
      out.xy.push(...run);
      out.edge.push(...run);
      const x1 = run[run.length - 2]!, x0 = run[0]!;
      out.xy.push(x1, base, x0, base);
    }
    run = [];
  };
  const push = (x: number) => run.push(x, clip(f(x, C)));
  /** Boundary between a and b, where ok(a) !== ok(b): the last x that is still inside. */
  const edge = (a: number, b: number, insideAtA: boolean) => {
    let inside = insideAtA ? a : b, outside = insideAtA ? b : a;
    for (let i = 0; i < BISECT; i++) {
      const m = (inside + outside) / 2;
      if (ok(m)) inside = m;
      else outside = m;
    }
    return inside;
  };
  const dx = (range.x1 - range.x0) / n;
  let prevX = range.x0, prevOk = ok(prevX);
  if (prevOk) push(prevX);
  for (let i = 1; i <= n; i++) {
    const x = range.x0 + i * dx;
    const cur = ok(x);
    if (cur !== prevOk) {
      const e = edge(prevX, x, prevOk);
      if (prevOk) {
        push(e);
        close();
      } else {
        push(e);
      }
    }
    if (cur) push(x);
    prevX = x;
    prevOk = cur;
  }
  close();
  out.rings.push(out.xy.length / 2);
  out.edgeOffsets.push(out.edge.length / 2);
  return out;
}

export interface RasterRect {
  x0: number;
  x1: number;
  y0: number;
  y1: number;
}

/** 1 where the region holds at the pixel center. Row 0 is the top (y1). */
export function rasterRegion(region: Region, rect: RasterRect, width: number, height: number, C: Float64Array): Uint8Array {
  const mask = new Uint8Array(width * height);
  const sx = (rect.x1 - rect.x0) / width, sy = (rect.y1 - rect.y0) / height;
  for (let j = 0; j < height; j++) {
    const y = rect.y1 - (j + 0.5) * sy;
    const row = j * width;
    for (let i = 0; i < width; i++) {
      if (region(rect.x0 + (i + 0.5) * sx, y, C)) mask[row + i] = 1;
    }
  }
  return mask;
}
