// Evaluation worker: compiles functions, samples them, returns geometry as
// Transferable typed arrays. Layer, order and style are attached on the main thread.

import type { Expr } from "../generated/Expr";
import { CodegenError, compile, compileRegion, type Compiled, type DrawnMembers, type FnSpec, type Region, type Scalar, type Vec } from "./codegen";
import { FeatureSet, type FeaturePt } from "./features";
import { computeFill, rasterRegion, type FillRule, type RasterRect } from "./fill";
import { clipLine, spanShape } from "./linalg";
import { makeRuntime } from "./numeric";
import { sampleImplicit } from "./implicit";
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
  /** Current scene: axis names (regions map them to x, y) and the clip region. */
  | { type: "scene"; axes: [string, string]; polar: boolean; clip: Expr | null }
  | { type: "sample"; id: number; keys: string[]; C: Float64Array; range: SampleRange; quality: Quality }
  | { type: "fills"; id: number; items: FillItem[]; C: Float64Array; range: SampleRange; quality: Quality }
  /** Numeric verification (stage 5c) over the given constant sets. */
  | { type: "verify"; id: number; specs: CheckSpec[]; sets: Float64Array[] };

export interface CurveResult {
  key: string;
  /**
   * `span`: a point (dim 0), a two-point polyline (dim 1) or a polygon ring covering the
   * computed area (dim 2). `arrow`: tail and head. `features`: the points in `points`.
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

export type FromWorker =
  | { type: "result"; id: number; quality: Quality; range: SampleRange; curves: CurveResult[]; warnings: WorkerWarning[] }
  | { type: "fills"; id: number; quality: Quality; range: SampleRange; results: (FillResult | RasterResult)[]; warnings: WorkerWarning[] }
  | { type: "checks"; id: number; results: CheckResult[] };

/** Scene budget for 2D curve samples (design.md "2층 장면 예산"). */
const SCENE_BUDGET = 1_000_000;

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
let axes: [string, string] = ["x", "y"];
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
  const env = { constIndex, fnIndex, axes };
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
      const r = c.kind === "Span" ? spanGeometry(key, c.vecs, C, range, origin) : arrowGeometry(key, c.vecs, C, origin);
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
      const [a, b] = c.family.krange(C);
      const lo = c.family.loStrict ? a! + 1 : a!, hi = c.family.hiStrict ? b! - 1 : b!;
      const parts: Pieces[] = [];
      truncated = false;
      if (!Number.isInteger(lo) || !Number.isInteger(hi)) {
        warnings.push({ source: key, code: "W-family-range", message: "family 범위 끝이 정수가 아니라 그리지 않음" });
      } else {
        const last = Math.min(hi, lo + MAX_FAMILY - 1);
        if (hi > last) warnings.push({ source: key, code: "W-family-cap", message: `family 반복이 ${MAX_FAMILY}개를 넘어 앞의 ${MAX_FAMILY}개만 그림` });
        for (let k = lo; k <= last; k++) parts.push(samplePieces(key, c.family.at(k), false, C, range, quality, warnings));
      }
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

/** `arrow(v)` from the origin, `arrow(v; P)` from P. */
function arrowGeometry(key: string, vecs: Vec[], C: Float64Array, origin: [number, number]): CurveResult | null {
  const v = vecs[0]?.(C);
  const p = vecs[1]?.(C) ?? [0, 0];
  if (!v || v.length !== 2 || p.length !== 2 || ![...v, ...p].every(Number.isFinite)) return null;
  const xy = [p[0]!, p[1]!, p[0]! + v[0]!, p[1]! + v[1]!];
  return { key, kind: "arrow", origin, coords: rel(xy, origin), offsets: Uint32Array.of(0, 2) };
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
