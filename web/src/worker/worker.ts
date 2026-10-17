// Evaluation worker: compiles functions, samples them, returns geometry as
// Transferable typed arrays. Layer, order and style are attached on the main thread.

import type { Expr } from "../generated/Expr";
import { CodegenError, compile, compileRegion, compileRegion3, type Compiled, type DrawnMembers, type FnSpec, type Region, type Region3, type Scalar, type Vec } from "./codegen";
import { FeatureSet, type FeaturePt } from "./features";
import { computeFill, rasterRegion, type FillRule, type RasterRect } from "./fill";
import { clipLine, spanShape } from "./linalg";
import { makeRuntime } from "./numeric";
import { sampleImplicit } from "./implicit";
import { boxOrigin, clipLine3, patchOf, planeInBox, sampleParamSurface, sampleSpaceCurve, sampleSurface, spanShape3, type Box3, type LineData, type MeshData, type PatchData } from "./mesh";
import { sampleParametric } from "./paramsample";
import { sampleCurve, sampleDiscrete, type Quality, type SampleRange } from "./sample";
import { runCheck, type CheckSpec } from "./verify";
import type { CheckResult } from "../generated/CheckResult";

/** A highlight the worker can draw: a function fill, or a targetless region. */
export interface HlSpec {
  key: string;
  /** Target function (fill); null = targetless region (raster). */
  fn: string | null;
  range: Expr | null;
}

export type FillItem =
  | { key: string; kind: "fill"; rule: FillRule; n: number }
  | { key: string; kind: "raster"; rect: RasterRect; width: number; height: number };

/** Key of the scene clip mask in `fills` requests and results. */
export const CLIP_KEY = "\u0000clip";

export type ToWorker =
  | { type: "functions"; fns: FnSpec[]; constKeys: string[] }
  | { type: "highlights"; specs: HlSpec[] }
  /** Current scene: axis names (regions map them to x, y; three for a 3D scene) and the clip region. */
  | { type: "scene"; axes: string[]; polar: boolean; clip: Expr | null }
  /** 3D scene (stage 6): geometry of `keys` over the view box. */
  | { type: "sample3d"; id: number; keys: string[]; C: Float64Array; box: Box3; quality: Quality }
  | { type: "sample"; id: number; keys: string[]; C: Float64Array; range: SampleRange; quality: Quality }
  | { type: "fills"; id: number; items: FillItem[]; C: Float64Array; range: SampleRange; quality: Quality }
  /** Numeric verification (stage 5c) over the given constant sets. */
  | { type: "verify"; id: number; specs: CheckSpec[]; sets: Float64Array[] };

export interface CurveResult {
  key: string;
  /**
   * `span`: a point (dim 0), a two-point polyline (dim 1) or a polygon ring covering the
   * computed area (dim 2). `arrow`: tail and head of each arrow (one per `family` copy,
   * each its own piece). `features`: the points in `points`.
   */
  kind: "polyline" | "points" | "span" | "arrow" | "features";
  /** Feature points found (features only). */
  points?: FeaturePt[];
  /** More than `MAX_FEATURE_POINTS` were found. */
  capped?: boolean;
  /** Span dimension drawn (span only). */
  dim?: 0 | 1 | 2;
  /** f64 origin; coords are relative to it. */
  origin: [number, number];
  coords: Float32Array;
  /** Piece starts plus final end (polyline only). */
  offsets: Uint32Array;
}

export interface WorkerWarning {
  source: string | null;
  code: string;
  message: string;
}

export interface FillResult {
  key: string;
  kind: "fill";
  origin: [number, number];
  /** Rings (relative to origin) and their offsets; one polygon per ring. */
  coords: Float32Array;
  rings: Uint32Array;
  edge: Float32Array;
  edgeOffsets: Uint32Array;
}

export interface RasterResult {
  key: string;
  kind: "raster";
  rect: RasterRect;
  width: number;
  height: number;
  mask: Uint8Array;
}

/** One 3D definition's geometry. */
export type Item3 =
  /** Surfaces and parametric surfaces (one mesh per `family` copy) and the patches of the highlights on them. */
  | { key: string; kind: "mesh"; meshes: MeshData[]; hls: { key: string; patch: PatchData }[] }
  /** Space curves. */
  | { key: string; kind: "lines"; line: LineData }
  /** dim 0: the origin; 1: the line's two ends in the box; 2: the plane's polygon in the box; 3: the whole box. */
  | { key: string; kind: "span"; dim: 0 | 1 | 2 | 3; origin: [number, number, number]; positions: Float32Array }
  /** Tail and head of each arrow (one per `family` copy). */
  | { key: string; kind: "arrow"; origin: [number, number, number]; positions: Float32Array };

export type FromWorker =
  | { type: "result"; id: number; quality: Quality; range: SampleRange; curves: CurveResult[]; warnings: WorkerWarning[] }
  | { type: "result3d"; id: number; quality: Quality; box: Box3; items: Item3[]; warnings: WorkerWarning[] }
  | { type: "fills"; id: number; quality: Quality; range: SampleRange; results: (FillResult | RasterResult)[]; warnings: WorkerWarning[] }
  | { type: "checks"; id: number; results: CheckResult[] };

/** Scene budget for 2D curve samples (design.md "2층 장면 예산"). */
const SCENE_BUDGET = 1_000_000;
/** Scene budget for 3D mesh vertices (design.md "2층 장면 예산"; measured in 6b). */
const MESH_BUDGET = 1_000_000;

const R = makeRuntime();
const F: Scalar[] = [];
let compiled = new Map<string, Compiled>();
let compileErrors = new Map<string, string>();
let constIndex = new Map<string, number>();
let constKeyList: string[] = [];
let fnIndex = new Map<string, number>();
let hlSpecs: HlSpec[] = [];
let regions = new Map<string, Region | null>();
let regionErrors = new Map<string, string>();
let axes: string[] = ["x", "y"];
/** 3D scenes: clip and surface highlight ranges over the three axes. */
let regions3 = new Map<string, Region3 | null>();
let polar = false;
let clipExpr: Expr | null = null;
const featureSet = new FeatureSet();

/**
 * A region written in (r, θ) evaluated on the Cartesian screen: a point belongs when
 * it holds for θ in [0, 2π) or the same angle one turn lower (`-\pi/2 <= \theta <= 0`).
 */
function polarRegion(reg: Region): Region {
  const TAU = 2 * Math.PI;
  return (x, y, C) => {
    const r = Math.hypot(x, y);
    let t = Math.atan2(y, x);
    if (t < 0) t += TAU;
    return reg(r, t, C) || reg(r, t - TAU, C);
  };
}

function setHighlights(specs: HlSpec[]): void {
  hlSpecs = specs;
  regions = new Map();
  regionErrors = new Map();
  regions3 = new Map();
  if (axes.length === 3) {
    const env = { constIndex, fnIndex };
    const all = clipExpr ? [...specs, { key: CLIP_KEY, fn: null, range: clipExpr }] : specs;
    for (const h of all) {
      try {
        regions3.set(h.key, h.range ? compileRegion3(h.range, axes, env, R, F) : null);
      } catch (e) {
        regionErrors.set(h.key, e instanceof CodegenError ? e.message : String(e));
      }
    }
    return;
  }
  const env = { constIndex, fnIndex, axes: axes as [string, string] };
  const all = clipExpr ? [...specs, { key: CLIP_KEY, fn: null, range: clipExpr }] : specs;
  for (const h of all) {
    try {
      const reg = h.range ? compileRegion(h.range, env, R, F) : null;
      regions.set(h.key, reg && polar ? polarRegion(reg) : reg);
    } catch (e) {
      regionErrors.set(h.key, e instanceof CodegenError ? e.message : String(e));
    }
  }
}

function fills(id: number, items: FillItem[], C: Float64Array, range: SampleRange, quality: Quality): Extract<FromWorker, { type: "fills" }> {
  const results: (FillResult | RasterResult)[] = [];
  const warnings: WorkerWarning[] = [];
  featureSet.run(C, range);
  const origin: [number, number] = [(range.x0 + range.x1) / 2, (range.y0 + range.y1) / 2];
  for (const it of items) {
    const err = regionErrors.get(it.key);
    const spec = it.key === CLIP_KEY ? { key: CLIP_KEY, fn: null, range: clipExpr } : hlSpecs.find((h) => h.key === it.key);
    if (err || !spec) {
      warnings.push({ source: it.key, code: "W-codegen", message: `계산 코드를 만들 수 없음 (${err ?? "정의 없음"})` });
      continue;
    }
    const region = regions.get(it.key) ?? null;
    if (it.kind === "raster") {
      if (!region) continue;
      results.push({ key: it.key, kind: "raster", rect: it.rect, width: it.width, height: it.height, mask: rasterRegion(region, it.rect, it.width, it.height, C) });
      continue;
    }
    const fn = spec.fn ? compiled.get(spec.fn) : undefined;
    if (!fn) continue;
    const g = computeFill(fn.scalar, region, it.rule, range, it.n, C);
    const rel = (xy: number[]) => {
      const out = new Float32Array(xy.length);
      for (let i = 0; i < xy.length; i += 2) {
        out[i] = xy[i]! - origin[0];
        out[i + 1] = xy[i + 1]! - origin[1];
      }
      return out;
    };
    results.push({ key: it.key, kind: "fill", origin, coords: rel(g.xy), rings: Uint32Array.from(g.rings), edge: rel(g.edge), edgeOffsets: Uint32Array.from(g.edgeOffsets) });
  }
  return { type: "fills", id, quality, range, results, warnings };
}

function setFunctions(fns: FnSpec[], constKeys: string[]): void {
  constIndex = new Map(constKeys.map((k, i) => [k, i]));
  constKeyList = constKeys;
  fnIndex = new Map(fns.map((f, i) => [f.key, i]));
  F.length = fns.length;
  // Discrete-ness flows from callees to callers.
  const own = new Map<string, boolean>();
  compiled = new Map();
  compileErrors = new Map();
  for (const [i, spec] of fns.entries()) {
    try {
      const c = compile(spec, { constIndex, fnIndex }, R, F, false);
      compiled.set(spec.key, c);
      own.set(spec.key, c.discrete);
      F[i] = c.table;
    } catch (e) {
      compileErrors.set(spec.key, e instanceof CodegenError ? e.message : String(e));
      F[i] = () => NaN;
    }
  }
  let changed = true;
  while (changed) {
    changed = false;
    for (const spec of fns) {
      const c = compiled.get(spec.key);
      if (c && !c.discrete && spec.calls.some((k) => compiled.get(k)?.discrete)) {
        c.discrete = true;
        changed = true;
      }
    }
  }
  featureSet.set(fns, { constIndex, fnIndex }, R, F, compiled, compileErrors);
}

function sample(id: number, keys: string[], C: Float64Array, range: SampleRange, quality: Quality): Extract<FromWorker, { type: "result" }> {
  const curves: CurveResult[] = [];
  const warnings: WorkerWarning[] = [];
  const origin: [number, number] = [(range.x0 + range.x1) / 2, (range.y0 + range.y1) / 2];
  let total = 0;
  featureSet.run(C, range);
  for (const key of keys) {
    const err = compileErrors.get(key);
    if (err) {
      warnings.push({ source: key, code: "W-codegen", message: `계산 코드를 만들 수 없음 (${err})` });
      continue;
    }
    const c = compiled.get(key);
    if (!c) continue;
    warnings.push(...featureSet.valueWarnings(key));
    if (c.kind === "Feature") {
      const out = featureSet.result(key);
      const coords = new Float32Array(out.points.length * 2);
      out.points.forEach((p, i) => {
        coords[2 * i] = p.x - origin[0];
        coords[2 * i + 1] = (Number.isFinite(p.y) ? p.y : origin[1]) - origin[1];
      });
      if (out.none) warnings.push({ source: key, code: "W-feature-none", message: out.none });
      if (out.capped) warnings.push({ source: key, code: "W-feature-cap", message: `특징점이 너무 많아 앞의 ${out.points.length}개만 그림 (범위를 좁히기)` });
      curves.push({ key, kind: "features", origin, coords, offsets: new Uint32Array(), points: out.points, capped: out.capped });
      continue;
    }
    if (c.kind === "Span" || c.kind === "Arrow") {
      const r = c.kind === "Span" ? spanGeometry(key, c.vecs, C, range, origin) : arrowGeometry(key, arrowCopies(key, c, C, warnings), C, origin);
      if (r) curves.push(r);
      else warnings.push({ source: key, code: "W-undefined", message: "벡터 값이 정의되지 않아 그리지 않음" });
      continue;
    }
    if (total >= SCENE_BUDGET) {
      warnings.push({ source: key, code: "W-budget", message: "장면 샘플 예산을 넘어 그리지 않음" });
      continue;
    }
    R.capped = false;
    R.overlap = false;
    let xs: Float64Array, ys: Float64Array, offsets: number[], truncated: boolean;
    let kind: CurveResult["kind"];
    if (c.family) {
      // One copy per integer k, joined as pieces of one polyline batch.
      const parts = familyCopies(key, c.family, C, warnings).map((m) => samplePieces(key, m, false, C, range, quality, warnings));
      truncated = false;
      const n = parts.reduce((s, p) => s + p.xs.length, 0);
      xs = new Float64Array(n);
      ys = new Float64Array(n);
      offsets = [];
      let at = 0;
      for (const p of parts) {
        xs.set(p.xs, at);
        ys.set(p.ys, at);
        for (let i = 0; i + 1 < p.offsets.length; i++) offsets.push(at + p.offsets[i]!);
        at += p.xs.length;
        truncated ||= p.truncated;
      }
      offsets.push(at);
      kind = "polyline";
    } else {
      ({ xs, ys, offsets, truncated, kind } = samplePieces(key, c, c.discrete, C, range, quality, warnings));
    }
    if (truncated) warnings.push({ source: key, code: "W-budget", message: "점이 너무 많아 품질을 낮춤" });
    if (R.capped) warnings.push({ source: key, code: "W-sum-terms", message: "sum/prod 항이 10만 개를 넘는 점은 그리지 않음" });
    if (R.overlap) warnings.push({ source: key, code: "W-split-overlap", message: "현재 값에서 split 구간이 겹치는 점이 있어 그 점은 정의 안 됨 (경계점 순서 확인)" });
    total += xs.length;
    const coords = new Float32Array(xs.length * 2);
    for (let i = 0; i < xs.length; i++) {
      coords[2 * i] = xs[i]! - origin[0];
      coords[2 * i + 1] = ys[i]! - origin[1];
    }
    curves.push({ key, kind, origin, coords, offsets: Uint32Array.from(offsets) });
  }
  return { type: "result", id, quality, range, curves, warnings };
}

/** Most copies one `family` definition draws. */
const MAX_FAMILY = 500;

/** The copies of a `family` definition at the current constants, at most `MAX_FAMILY`. */
function familyCopies(key: string, fam: NonNullable<Compiled["family"]>, C: Float64Array, warnings: WorkerWarning[]): DrawnMembers[] {
  const [a, b] = fam.krange(C);
  const lo = fam.loStrict ? a! + 1 : a!, hi = fam.hiStrict ? b! - 1 : b!;
  if (!Number.isInteger(lo) || !Number.isInteger(hi)) {
    warnings.push({ source: key, code: "W-family-range", message: "family 범위 끝이 정수가 아니라 그리지 않음" });
    return [];
  }
  const last = Math.min(hi, lo + MAX_FAMILY - 1);
  if (hi > last) warnings.push({ source: key, code: "W-family-cap", message: `family 반복이 ${MAX_FAMILY}개를 넘어 앞의 ${MAX_FAMILY}개만 그림` });
  const out: DrawnMembers[] = [];
  for (let k = lo; k <= last; k++) out.push(fam.at(k));
  return out;
}

/** An arrow's vectors (vector, start point), one set per copy of a `family`. */
function arrowCopies(key: string, c: Compiled, C: Float64Array, warnings: WorkerWarning[]): Vec[][] {
  return c.family ? familyCopies(key, c.family, C, warnings).map((m) => m.vecs ?? []) : [c.vecs];
}

/**
 * Tail and head of each arrow (`dim` coordinates each), skipping undefined ones.
 * Null when there were arrows and none is defined.
 */
function arrowPairs(copies: Vec[][], C: Float64Array, dim: 2 | 3): number[] | null {
  const out: number[] = [];
  for (const vecs of copies) {
    const v = vecs[0]?.(C);
    const p = vecs[1]?.(C) ?? new Array<number>(dim).fill(0);
    if (!v || v.length !== dim || p.length !== dim || ![...v, ...p].every(Number.isFinite)) continue;
    out.push(...p, ...p.map((x, i) => x + v[i]!));
  }
  return out.length || !copies.length ? out : null;
}

interface Pieces {
  xs: Float64Array;
  ys: Float64Array;
  offsets: number[];
  truncated: boolean;
  kind: "polyline" | "points";
}

/** Samples one drawn curve: parametric / polar, implicit, discrete, or y = f(x). */
function samplePieces(key: string, m: DrawnMembers, discrete: boolean, C: Float64Array, range: SampleRange, quality: Quality, warnings: WorkerWarning[]): Pieces {
  if (m.param) {
    const [t0, t1] = m.trange ? m.trange(C) : [0, 2 * Math.PI];
    if (!(t0! <= t1!)) warnings.push({ source: key, code: "W-range", message: "매개변수 범위가 비어 있거나 정의되지 않아 그리지 않음" });
    return { ...sampleParametric(m.param, C, t0!, t1!, range, quality), kind: "polyline" };
  }
  if (m.implicit) return { ...sampleImplicit(m.implicit, C, range, quality), kind: "polyline" };
  if (discrete) {
    const d = sampleDiscrete(m.batch!, C, range);
    return { ...d, offsets: [0, d.xs.length], kind: "points" };
  }
  return { ...sampleCurve(m.batch!, m.scalar!, C, range, quality), kind: "polyline" };
}

function rel(xy: number[], origin: [number, number]): Float32Array {
  const out = new Float32Array(xy.length);
  for (let i = 0; i < xy.length; i += 2) {
    out[i] = xy[i]! - origin[0];
    out[i + 1] = xy[i + 1]! - origin[1];
  }
  return out;
}

/** Point, line through the origin clipped to the computed area, or the whole area. */
function spanGeometry(key: string, vecs: Vec[], C: Float64Array, range: SampleRange, origin: [number, number]): CurveResult | null {
  const shape = spanShape(vecs.map((v) => v(C)));
  if (shape.dim === null) return null;
  const box = { x0: range.x0, x1: range.x1, y0: range.y0, y1: range.y1 };
  let xy: number[] = [];
  if (shape.dim === 0) xy = [0, 0];
  else if (shape.dim === 1) {
    // Many points along the line so the name label finds a spot on screen.
    const seg = clipLine(shape.dir!, box);
    const n = 64;
    if (seg) for (let i = 0; i <= n; i++) xy.push(seg[0] + ((seg[2] - seg[0]) * i) / n, seg[1] + ((seg[3] - seg[1]) * i) / n);
  }
  else xy = [box.x0, box.y0, box.x1, box.y0, box.x1, box.y1, box.x0, box.y1];
  return { key, kind: "span", dim: shape.dim, origin, coords: rel(xy, origin), offsets: Uint32Array.of(0, xy.length / 2) };
}

/** `arrow(v)` from the origin, `arrow(v; P)` from P; a `family` gives one per copy. */
function arrowGeometry(key: string, copies: Vec[][], C: Float64Array, origin: [number, number]): CurveResult | null {
  const xy = arrowPairs(copies, C, 2);
  if (!xy) return null;
  // One two-point piece per arrow (hover snaps along each).
  const offsets = Uint32Array.from({ length: xy.length / 4 + 1 }, (_, i) => 2 * i);
  return { key, kind: "arrow", origin, coords: rel(xy, origin), offsets };
}

/** Geometry of 3D definitions over the box (stage 6a). */
function sample3d(id: number, keys: string[], C: Float64Array, box: Box3, quality: Quality): Extract<FromWorker, { type: "result3d" }> {
  const items: Item3[] = [];
  const warnings: WorkerWarning[] = [];
  const clip = regions3.get(CLIP_KEY) ?? null;
  if (regionErrors.has(CLIP_KEY)) warnings.push({ source: null, code: "W-codegen", message: `clip 계산 코드를 만들 수 없음 (${regionErrors.get(CLIP_KEY)})` });
  const origin = boxOrigin(box);
  let vertices = 0;
  for (const key of keys) {
    const err = compileErrors.get(key);
    if (err) {
      warnings.push({ source: key, code: "W-codegen", message: `계산 코드를 만들 수 없음 (${err})` });
      continue;
    }
    const c = compiled.get(key);
    if (!c) continue;
    if (c.kind === "Span") {
      const shape = spanShape3(c.vecs.map((v) => v(C)));
      if (shape.dim === null) {
        warnings.push({ source: key, code: "W-undefined", message: "벡터 값이 정의되지 않아 그리지 않음" });
        continue;
      }
      let pts: number[] = [];
      if (shape.dim === 0) pts = [0, 0, 0];
      else if (shape.dim === 1) pts = clipLine3(shape.dir!, box) ?? [];
      else if (shape.dim === 2) pts = planeInBox(shape.dir!, box).flat();
      items.push({ key, kind: "span", dim: shape.dim, origin, positions: Float32Array.from(pts.map((v, i) => v - origin[i % 3]!)) });
      continue;
    }
    if (c.kind === "Arrow") {
      const xyz = arrowPairs(arrowCopies(key, c, C, warnings), C, 3);
      if (!xyz) {
        warnings.push({ source: key, code: "W-undefined", message: "벡터 값이 정의되지 않아 그리지 않음" });
        continue;
      }
      items.push({ key, kind: "arrow", origin, positions: Float32Array.from(xyz.map((x, i) => x - origin[i % 3]!)) });
      continue;
    }
    if (vertices >= MESH_BUDGET) {
      warnings.push({ source: key, code: "W-budget", message: "장면 정점 예산을 넘어 그리지 않음" });
      continue;
    }
    const copies: DrawnMembers[] = c.family ? familyCopies(key, c.family, C, warnings) : [c];
    R.capped = false;
    R.overlap = false;
    const meshes: MeshData[] = [];
    const lines: LineData[] = [];
    for (const m of copies) {
      if (m.surface) meshes.push(sampleSurface(m.surface, C, box, quality, clip));
      else if (m.param2) {
        const u = m.trange!(C), v = m.trange2!(C);
        if (!(u[0] <= u[1]) || !(v[0] <= v[1])) warnings.push({ source: key, code: "W-range", message: "매개변수 범위가 비어 있거나 정의되지 않아 그리지 않음" });
        else meshes.push(sampleParamSurface(m.param2, C, u, v, box, quality, clip));
      } else if (m.param) {
        const [t0, t1] = m.trange ? m.trange(C) : [0, 2 * Math.PI];
        if (!(t0! <= t1!)) warnings.push({ source: key, code: "W-range", message: "매개변수 범위가 비어 있거나 정의되지 않아 그리지 않음" });
        else lines.push(sampleSpaceCurve(m.param, C, t0!, t1!, box, quality, clip));
      }
    }
    if (R.capped) warnings.push({ source: key, code: "W-sum-terms", message: "sum/prod 항이 10만 개를 넘는 점은 그리지 않음" });
    if (R.overlap) warnings.push({ source: key, code: "W-split-overlap", message: "현재 값에서 split 구간이 겹치는 점이 있어 그 점은 정의 안 됨 (경계점 순서 확인)" });
    if (meshes.length) {
      vertices += meshes.reduce((s, m) => s + m.positions.length / 3, 0);
      const hls = hlSpecs
        .filter((h) => h.fn === key && meshes.length === 1)
        .flatMap((h) => {
          const reg = regions3.get(h.key);
          if (regionErrors.has(h.key)) {
            warnings.push({ source: h.key, code: "W-codegen", message: `계산 코드를 만들 수 없음 (${regionErrors.get(h.key)})` });
            return [];
          }
          return [{ key: h.key, patch: patchOf(meshes[0]!, reg ?? null, C) }];
        });
      items.push({ key, kind: "mesh", meshes, hls });
    } else if (lines.length) {
      // Family copies join as pieces of one line batch.
      const n = lines.reduce((s, l) => s + l.positions.length, 0);
      const positions = new Float32Array(n);
      const offsets: number[] = [];
      let at = 0;
      for (const l of lines) {
        positions.set(l.positions, at * 3);
        for (let i = 0; i + 1 < l.offsets.length; i++) offsets.push(at + l.offsets[i]!);
        at += l.positions.length / 3;
      }
      offsets.push(at);
      vertices += at;
      items.push({ key, kind: "lines", line: { origin, positions, offsets: Uint32Array.from(offsets) } });
    }
  }
  return { type: "result3d", id, quality, box, items, warnings };
}

function transfer3(items: Item3[]): ArrayBuffer[] {
  return items.flatMap((it): ArrayBuffer[] => {
    switch (it.kind) {
      case "mesh":
        return [...it.meshes.flatMap((m) => [m.positions.buffer, m.normals.buffer, m.alpha.buffer, m.indices.buffer, m.wire.buffer]), ...it.hls.flatMap((h) => [h.patch.positions.buffer, h.patch.normals.buffer, h.patch.alpha.buffer])] as ArrayBuffer[];
      case "lines":
        return [it.line.positions.buffer, it.line.offsets.buffer] as ArrayBuffer[];
      default:
        return [it.positions.buffer] as ArrayBuffer[];
    }
  });
}

self.onmessage = (ev: MessageEvent<ToWorker>) => {
  const m = ev.data;
  if (m.type === "functions") {
    setFunctions(m.fns, m.constKeys);
    // Regions index the same constant and function tables.
    setHighlights(hlSpecs);
  } else if (m.type === "highlights") {
    setHighlights(m.specs);
  } else if (m.type === "scene") {
    axes = m.axes;
    polar = m.polar;
    clipExpr = m.clip;
    setHighlights(hlSpecs);
  } else if (m.type === "verify") {
    const env = { env: { constIndex, fnIndex }, R, F, constKeys: constKeyList };
    const results = m.specs.map((spec) => {
      try {
        return runCheck(spec, m.sets, env);
      } catch {
        return { key: spec.key, pair: spec.pair, kind: spec.kind, a: spec.a, b: spec.b, order: spec.order, status: "Unverifiable", points: 0, one_sided: 0, counter: null } satisfies CheckResult;
      }
    });
    (self as unknown as Worker).postMessage({ type: "checks", id: m.id, results } satisfies FromWorker);
  } else if (m.type === "sample3d") {
    const r = sample3d(m.id, m.keys, m.C, m.box, m.quality);
    (self as unknown as Worker).postMessage(r, transfer3(r.items));
  } else if (m.type === "fills") {
    const r = fills(m.id, m.items, m.C, m.range, m.quality);
    const transfer = r.results.flatMap((x) => (x.kind === "fill" ? [x.coords.buffer, x.rings.buffer, x.edge.buffer, x.edgeOffsets.buffer] : [x.mask.buffer]));
    (self as unknown as Worker).postMessage(r, transfer);
  } else {
    const r = sample(m.id, m.keys, m.C, m.range, m.quality);
    const transfer = r.curves.flatMap((c) => [c.coords.buffer, c.offsets.buffer]);
    (self as unknown as Worker).postMessage(r, transfer);
  }
};
