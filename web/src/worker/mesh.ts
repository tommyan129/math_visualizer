// 3D geometry (stage 6a): surfaces over the view box, parametric surfaces and space
// curves, spans and arrows of length-3 vectors. Pure functions over compiled members;
// coordinates are math coordinates relative to `origin` (the box center).

import type { Param, Param2, Region3, Surface } from "./codegen";

/** The 3D view box: one range per scene axis. */
export interface Box3 {
  x0: number;
  x1: number;
  y0: number;
  y1: number;
  z0: number;
  z1: number;
}

export type Quality = "coarse" | "fine";

/** Grid cells per side of a surface / parametric surface. */
export const GRID = { coarse: 48, fine: 160 } as const;
/** Samples of a space curve. */
export const CURVE_SAMPLES = { coarse: 600, fine: 3000 } as const;
/** Faint mesh lines: about this many per side. */
const WIRE_LINES = 16;

export interface MeshData {
  origin: [number, number, number];
  /** xyz per vertex, relative to origin; non-finite vertices are moved to the origin. */
  positions: Float32Array;
  normals: Float32Array;
  /** 1 = drawn, 0 = cut by the clip (interpolated: the cut runs between vertices). */
  alpha: Float32Array;
  /** Triangles with every vertex defined (and no jump across the box height). */
  indices: Uint32Array;
  /** Faint mesh lines as segment pairs (xyz xyz). */
  wire: Float32Array;
  /** Vertices per side: (n + 1) × (m + 1) grid, row-major in the second parameter. */
  n: number;
  m: number;
}

export interface LineData {
  origin: [number, number, number];
  /** xyz per point, relative to origin. */
  positions: Float32Array;
  /** Piece starts plus final end. */
  offsets: Uint32Array;
}

export function boxOrigin(b: Box3): [number, number, number] {
  return [(b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2, (b.z0 + b.z1) / 2];
}

/** Points of a grid over (s, t) ∈ [s0, s1] × [t0, t1], (n + 1) × (m + 1), row-major in t. */
function grid(n: number, m: number, s0: number, s1: number, t0: number, t1: number, at: (s: number, t: number, out: number[]) => void): Float64Array {
  const out = new Float64Array((n + 1) * (m + 1) * 3);
  const p = [0, 0, 0];
  for (let j = 0; j <= m; j++) {
    const t = t0 + ((t1 - t0) * j) / m;
    for (let i = 0; i <= n; i++) {
      const s = s0 + ((s1 - s0) * i) / n;
      at(s, t, p);
      const k = (j * (n + 1) + i) * 3;
      out[k] = p[0]!;
      out[k + 1] = p[1]!;
      out[k + 2] = p[2]!;
    }
  }
  return out;
}

/** `g(x, y)` over the box's x, y ranges. */
export function sampleSurface(g: Surface, C: Float64Array, box: Box3, quality: Quality, clip: Region3 | null): MeshData {
  const n = GRID[quality];
  const pts = grid(n, n, box.x0, box.x1, box.y0, box.y1, (x, y, out) => {
    out[0] = x;
    out[1] = y;
    out[2] = g(x, y, C);
  });
  return meshOf(pts, n, n, C, box, clip);
}

/** `s(u, v)` over its parameter ranges. */
export function sampleParamSurface(s: Param2, C: Float64Array, u: [number, number], v: [number, number], box: Box3, quality: Quality, clip: Region3 | null): MeshData {
  const n = GRID[quality];
  const pts = grid(n, n, u[0], u[1], v[0], v[1], (a, b, out) => {
    const p = s(a, b, C);
    out[0] = p[0] ?? NaN;
    out[1] = p[1] ?? NaN;
    out[2] = p[2] ?? NaN;
  });
  return meshOf(pts, n, n, C, box, clip);
}

const finite3 = (p: Float64Array, k: number) => Number.isFinite(p[k]!) && Number.isFinite(p[k + 1]!) && Number.isFinite(p[k + 2]!);

/**
 * Triangles, normals and the clip of a sampled grid. A cell is left out when a corner is
 * undefined, or when its z values span more than the box height (a jump such as 1/x
 * between two grid points, which would otherwise draw a wall).
 */
function meshOf(pts: Float64Array, n: number, m: number, C: Float64Array, box: Box3, clip: Region3 | null): MeshData {
  const origin = boxOrigin(box);
  const count = (n + 1) * (m + 1);
  const positions = new Float32Array(count * 3);
  const normals = new Float32Array(count * 3);
  const alpha = new Float32Array(count);
  const ok = new Uint8Array(count);
  for (let v = 0; v < count; v++) {
    const k = v * 3;
    if (!finite3(pts, k)) continue;
    ok[v] = 1;
    positions[k] = pts[k]! - origin[0];
    positions[k + 1] = pts[k + 1]! - origin[1];
    positions[k + 2] = pts[k + 2]! - origin[2];
    alpha[v] = !clip || clip(pts[k]!, pts[k + 1]!, pts[k + 2]!, C) ? 1 : 0;
  }
  const idx = (i: number, j: number) => j * (n + 1) + i;
  // Normals: cross product of the differences along both grid directions (one-sided at
  // the edges and next to undefined points).
  const d = (a: number, b: number, out: number[]) => {
    out[0] = pts[b * 3]! - pts[a * 3]!;
    out[1] = pts[b * 3 + 1]! - pts[a * 3 + 1]!;
    out[2] = pts[b * 3 + 2]! - pts[a * 3 + 2]!;
  };
  const du = [0, 0, 0], dv = [0, 0, 0];
  for (let j = 0; j <= m; j++) {
    for (let i = 0; i <= n; i++) {
      const c = idx(i, j);
      if (!ok[c]) continue;
      const l = i > 0 && ok[idx(i - 1, j)] ? idx(i - 1, j) : c, r = i < n && ok[idx(i + 1, j)] ? idx(i + 1, j) : c;
      const b = j > 0 && ok[idx(i, j - 1)] ? idx(i, j - 1) : c, t = j < m && ok[idx(i, j + 1)] ? idx(i, j + 1) : c;
      d(l, r, du);
      d(b, t, dv);
      let nx = du[1]! * dv[2]! - du[2]! * dv[1]!, ny = du[2]! * dv[0]! - du[0]! * dv[2]!, nz = du[0]! * dv[1]! - du[1]! * dv[0]!;
      const len = Math.hypot(nx, ny, nz);
      if (len > 0) {
        nx /= len;
        ny /= len;
        nz /= len;
      } else {
        nx = 0;
        ny = 0;
        nz = 1;
      }
      normals[c * 3] = nx;
      normals[c * 3 + 1] = ny;
      normals[c * 3 + 2] = nz;
    }
  }
  const height = box.z1 - box.z0;
  const tri: number[] = [];
  for (let j = 0; j < m; j++) {
    for (let i = 0; i < n; i++) {
      const a = idx(i, j), b = idx(i + 1, j), c = idx(i + 1, j + 1), e = idx(i, j + 1);
      if (!ok[a] || !ok[b] || !ok[c] || !ok[e]) continue;
      const za = pts[a * 3 + 2]!, zb = pts[b * 3 + 2]!, zc = pts[c * 3 + 2]!, ze = pts[e * 3 + 2]!;
      if (Math.max(za, zb, zc, ze) - Math.min(za, zb, zc, ze) > height) continue;
      tri.push(a, b, c, a, c, e);
    }
  }
  // Mesh lines along both directions, about WIRE_LINES per side, between drawn vertices.
  const wire: number[] = [];
  const seg = (a: number, b: number) => {
    if (!ok[a] || !ok[b] || !alpha[a] || !alpha[b]) return;
    if (Math.abs(pts[a * 3 + 2]! - pts[b * 3 + 2]!) > height) return;
    for (const v of [a, b]) wire.push(positions[v * 3]!, positions[v * 3 + 1]!, positions[v * 3 + 2]!);
  };
  const si = Math.max(1, Math.round(n / WIRE_LINES)), sj = Math.max(1, Math.round(m / WIRE_LINES));
  for (let j = 0; j <= m; j += sj) for (let i = 0; i < n; i++) seg(idx(i, j), idx(i + 1, j));
  for (let i = 0; i <= n; i += si) for (let j = 0; j < m; j++) seg(idx(i, j), idx(i, j + 1));
  return { origin, positions, normals, alpha, indices: Uint32Array.from(tri), wire: Float32Array.from(wire), n, m };
}

/** The part of a surface a highlight covers: its own triangles, carrying the clip alpha. */
export interface PatchData {
  origin: [number, number, number];
  /** xyz per vertex, relative to origin; three vertices per triangle. */
  positions: Float32Array;
  normals: Float32Array;
  /** The mesh's clip alpha, interpolated (the clip still cuts per vertex). */
  alpha: Float32Array;
}

/** Bisection steps for a region boundary on a triangle edge (1/256 of the edge). */
const EDGE_STEPS = 8;

/**
 * The triangles of a mesh inside a region (surface highlights; null = the whole mesh).
 * Triangles across the boundary are cut where it crosses their edges, found by
 * bisection along the edge, so the patch edge follows the region instead of running
 * through edge midpoints (a 0/1 vertex mask cut at 0.5 drew a zigzag polygon). A region
 * narrower than a grid cell between two outside vertices is still missed.
 */
export function patchOf(mesh: MeshData, region: Region3 | null, C: Float64Array): PatchData {
  const { origin, positions: P, normals: N, alpha: A, indices } = mesh;
  const [ox, oy, oz] = origin;
  const count = P.length / 3;
  const inside = new Uint8Array(count);
  for (let v = 0; v < count; v++) inside[v] = !region || region(P[v * 3]! + ox, P[v * 3 + 1]! + oy, P[v * 3 + 2]! + oz, C) ? 1 : 0;
  const pos: number[] = [], nrm: number[] = [], alp: number[] = [];
  // A point on the edge a → b at fraction t.
  type Pt = { a: number; b: number; t: number };
  const put = (q: Pt) => {
    const { a, b, t } = q;
    for (let c = 0; c < 3; c++) pos.push(P[a * 3 + c]! + (P[b * 3 + c]! - P[a * 3 + c]!) * t);
    let nx = N[a * 3]! + (N[b * 3]! - N[a * 3]!) * t, ny = N[a * 3 + 1]! + (N[b * 3 + 1]! - N[a * 3 + 1]!) * t, nz = N[a * 3 + 2]! + (N[b * 3 + 2]! - N[a * 3 + 2]!) * t;
    const len = Math.hypot(nx, ny, nz) || 1;
    nrm.push(nx / len, ny / len, nz / len);
    alp.push(A[a]! + (A[b]! - A[a]!) * t);
  };
  const at = (v: number): Pt => ({ a: v, b: v, t: 0 });
  // Boundary between an inside vertex a and an outside vertex b.
  const cross = (a: number, b: number): Pt => {
    let lo = 0, hi = 1;
    for (let k = 0; k < EDGE_STEPS; k++) {
      const t = (lo + hi) / 2;
      const x = P[a * 3]! + (P[b * 3]! - P[a * 3]!) * t + ox, y = P[a * 3 + 1]! + (P[b * 3 + 1]! - P[a * 3 + 1]!) * t + oy, z = P[a * 3 + 2]! + (P[b * 3 + 2]! - P[a * 3 + 2]!) * t + oz;
      if (region!(x, y, z, C)) lo = t;
      else hi = t;
    }
    return { a, b, t: (lo + hi) / 2 };
  };
  const tri = (p: Pt, q: Pt, r: Pt) => {
    put(p);
    put(q);
    put(r);
  };
  for (let i = 0; i < indices.length; i += 3) {
    const v = [indices[i]!, indices[i + 1]!, indices[i + 2]!];
    const n = inside[v[0]!]! + inside[v[1]!]! + inside[v[2]!]!;
    if (n === 0) continue;
    if (n === 3) {
      tri(at(v[0]!), at(v[1]!), at(v[2]!));
      continue;
    }
    // Rotate so the odd vertex comes first (keeps the winding).
    const odd = n === 1 ? v.findIndex((x) => inside[x]) : v.findIndex((x) => !inside[x]);
    const a = v[odd]!, b = v[(odd + 1) % 3]!, c = v[(odd + 2) % 3]!;
    if (n === 1) tri(at(a), cross(a, b), cross(a, c));
    else {
      const ba = cross(b, a), ca = cross(c, a);
      tri(ba, at(b), at(c));
      tri(ba, at(c), ca);
    }
  }
  return { origin, positions: Float32Array.from(pos), normals: Float32Array.from(nrm), alpha: Float32Array.from(alp) };
}

/**
 * Space curve over [t0, t1]: evenly spaced samples, broken where a point is undefined,
 * outside the clip, or jumps by more than the box diagonal between two samples.
 */
export function sampleSpaceCurve(p: Param, C: Float64Array, t0: number, t1: number, box: Box3, quality: Quality, clip: Region3 | null): LineData {
  const n = CURVE_SAMPLES[quality];
  const origin = boxOrigin(box);
  const diag = Math.hypot(box.x1 - box.x0, box.y1 - box.y0, box.z1 - box.z0);
  const xyz: number[] = [];
  const offsets: number[] = [];
  let open = false;
  let prev: number[] | null = null;
  for (let i = 0; i <= n; i++) {
    const t = t0 + ((t1 - t0) * i) / n;
    const q = p(t, C);
    const good = q.length === 3 && q.every(Number.isFinite) && (!clip || clip(q[0]!, q[1]!, q[2]!, C));
    const jump = good && prev !== null && Math.hypot(q[0]! - prev[0]!, q[1]! - prev[1]!, q[2]! - prev[2]!) > diag;
    if (!good || jump) {
      if (open && offsets.length && xyz.length / 3 - offsets[offsets.length - 1]! < 2) {
        // A lone point is not a piece.
        xyz.length = offsets.pop()! * 3;
      }
      open = false;
    }
    if (good) {
      if (!open) offsets.push(xyz.length / 3);
      open = true;
      xyz.push(q[0]! - origin[0], q[1]! - origin[1], q[2]! - origin[2]);
    }
    prev = good ? q : null;
  }
  if (open && xyz.length / 3 - offsets[offsets.length - 1]! < 2) xyz.length = offsets.pop()! * 3;
  offsets.push(xyz.length / 3);
  return { origin, positions: Float32Array.from(xyz), offsets: Uint32Array.from(offsets) };
}

// ----- spans --------------------------------------------------------------

/** Same tolerances as the 2D span (`linalg.ts`). */
const PARALLEL_TOL = 1e-9;
const ZERO_TOL = 1e-12;

export interface Span3 {
  /** 0 = origin, 1 = line, 2 = plane, 3 = all of space; null = not a finite number. */
  dim: 0 | 1 | 2 | 3 | null;
  /** Line direction (dim 1) or plane normal (dim 2), unit length. */
  dir: [number, number, number] | null;
}

const cross = (a: number[], b: number[]): [number, number, number] => [a[1]! * b[2]! - a[2]! * b[1]!, a[2]! * b[0]! - a[0]! * b[2]!, a[0]! * b[1]! - a[1]! * b[0]!];
const norm = (a: number[]) => Math.hypot(a[0]!, a[1]!, a[2]!);

/** Shape of the span of length-3 vectors at their current values. */
export function spanShape3(vecs: number[][]): Span3 {
  if (vecs.some((v) => v.length !== 3 || !v.every(Number.isFinite))) return { dim: null, dir: null };
  const scale = Math.max(0, ...vecs.flat().map(Math.abs));
  const nonzero = vecs.filter((v) => norm(v) > ZERO_TOL * scale);
  if (scale === 0 || nonzero.length === 0) return { dim: 0, dir: null };
  const d = nonzero[0]!;
  const nd = norm(d);
  let plane: [number, number, number] | null = null;
  let w0: number[] | null = null;
  for (const w of nonzero.slice(1)) {
    const c = cross(d, w);
    if (norm(c) > PARALLEL_TOL * nd * norm(w)) {
      plane = c;
      w0 = w;
      break;
    }
  }
  if (!plane) return { dim: 1, dir: [d[0]! / nd, d[1]! / nd, d[2]! / nd] };
  const np = norm(plane);
  for (const w of nonzero) {
    if (w === d || w === w0) continue;
    const vol = plane[0] * w[0]! + plane[1] * w[1]! + plane[2] * w[2]!;
    if (Math.abs(vol) > PARALLEL_TOL * np * norm(w)) return { dim: 3, dir: null };
  }
  return { dim: 2, dir: [plane[0] / np, plane[1] / np, plane[2] / np] };
}

/** The part of the line t·dir inside the box (slab method), or null. */
export function clipLine3(dir: [number, number, number], box: Box3): [number, number, number, number, number, number] | null {
  let lo = -Infinity, hi = Infinity;
  const ranges = [[box.x0, box.x1], [box.y0, box.y1], [box.z0, box.z1]] as const;
  for (let a = 0; a < 3; a++) {
    const d = dir[a]!;
    const [p, q] = ranges[a]!;
    if (d === 0) {
      if (0 < p || 0 > q) return null;
      continue;
    }
    const t1 = p / d, t2 = q / d;
    lo = Math.max(lo, Math.min(t1, t2));
    hi = Math.min(hi, Math.max(t1, t2));
  }
  if (!(hi > lo)) return null;
  return [lo * dir[0], lo * dir[1], lo * dir[2], hi * dir[0], hi * dir[1], hi * dir[2]];
}

/**
 * The plane through the origin with normal `nrm`, cut to the box: a convex polygon
 * (its vertices in order around the normal), empty when the plane misses the box.
 */
export function planeInBox(nrm: [number, number, number], box: Box3): number[][] {
  const xs = [box.x0, box.x1], ys = [box.y0, box.y1], zs = [box.z0, box.z1];
  const corners: number[][] = [];
  for (const x of xs) for (const y of ys) for (const z of zs) corners.push([x, y, z]);
  const side = corners.map((c) => nrm[0] * c[0]! + nrm[1] * c[1]! + nrm[2] * c[2]!);
  const pts: number[][] = [];
  // Box edges: corner pairs differing in one coordinate.
  for (let a = 0; a < 8; a++) {
    for (let b = a + 1; b < 8; b++) {
      const diff = [0, 1, 2].filter((k) => corners[a]![k] !== corners[b]![k]).length;
      if (diff !== 1) continue;
      const sa = side[a]!, sb = side[b]!;
      if (sa === 0) pts.push(corners[a]!);
      if ((sa < 0 && sb > 0) || (sa > 0 && sb < 0)) {
        const t = sa / (sa - sb);
        pts.push(corners[a]!.map((v, k) => v + (corners[b]![k]! - v) * t));
      }
    }
  }
  if (pts.length < 3) return [];
  const c = [0, 1, 2].map((k) => pts.reduce((s, p) => s + p[k]!, 0) / pts.length);
  // Order by angle in the plane.
  const ref = norm([1, 0, 0].map((v, k) => v - nrm[0] * nrm[k]!)) > 0.1 ? [1, 0, 0] : [0, 1, 0];
  const e1 = (() => {
    const d = ref[0]! * nrm[0] + ref[1]! * nrm[1] + ref[2]! * nrm[2];
    const v = ref.map((r, k) => r - d * nrm[k]!);
    const l = norm(v);
    return v.map((x) => x / l);
  })();
  const e2 = cross(nrm, e1);
  const angle = (p: number[]) => {
    const d = p.map((v, k) => v - c[k]!);
    return Math.atan2(d[0]! * e2[0] + d[1]! * e2[1] + d[2]! * e2[2], d[0]! * e1[0]! + d[1]! * e1[1]! + d[2]! * e1[2]!);
  };
  const out = pts.sort((a, b) => angle(a) - angle(b));
  // Drop repeated points (a corner on the plane is found from several edges).
  return out.filter((p, i) => i === 0 || norm(p.map((v, k) => v - out[i - 1]![k]!)) > 1e-12 * (1 + norm(p)));
}
