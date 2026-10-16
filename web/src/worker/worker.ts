// Evaluation worker: compiles functions, samples them, returns geometry as
// Transferable typed arrays. Layer, order and style are attached on the main thread.

import type { Expr } from "../generated/Expr";
import { CodegenError, compile, compileRegion, type Compiled, type FnSpec, type Region, type Scalar } from "./codegen";
import { computeFill, rasterRegion, type FillRule, type RasterRect } from "./fill";
import { makeRuntime } from "./numeric";
import { sampleCurve, sampleDiscrete, type Quality, type SampleRange } from "./sample";

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
  | { type: "scene"; axes: [string, string]; clip: Expr | null }
  | { type: "sample"; id: number; keys: string[]; C: Float64Array; range: SampleRange; quality: Quality }
  | { type: "fills"; id: number; items: FillItem[]; C: Float64Array; range: SampleRange; quality: Quality };

export interface CurveResult {
  key: string;
  kind: "polyline" | "points";
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
  | { type: "fills"; id: number; quality: Quality; range: SampleRange; results: (FillResult | RasterResult)[]; warnings: WorkerWarning[] };

/** Scene budget for 2D curve samples (design.md "2층 장면 예산"). */
const SCENE_BUDGET = 1_000_000;

const R = makeRuntime();
const F: Scalar[] = [];
let compiled = new Map<string, Compiled>();
let compileErrors = new Map<string, string>();
let constIndex = new Map<string, number>();
let fnIndex = new Map<string, number>();
let hlSpecs: HlSpec[] = [];
let regions = new Map<string, Region | null>();
let regionErrors = new Map<string, string>();
let axes: [string, string] = ["x", "y"];
let clipExpr: Expr | null = null;

function setHighlights(specs: HlSpec[]): void {
  hlSpecs = specs;
  regions = new Map();
  regionErrors = new Map();
  const env = { constIndex, fnIndex, axes };
  const all = clipExpr ? [...specs, { key: CLIP_KEY, fn: null, range: clipExpr }] : specs;
  for (const h of all) {
    try {
      regions.set(h.key, h.range ? compileRegion(h.range, env, R, F) : null);
    } catch (e) {
      regionErrors.set(h.key, e instanceof CodegenError ? e.message : String(e));
    }
  }
}

function fills(id: number, items: FillItem[], C: Float64Array, range: SampleRange, quality: Quality): Extract<FromWorker, { type: "fills" }> {
  const results: (FillResult | RasterResult)[] = [];
  const warnings: WorkerWarning[] = [];
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
      F[i] = c.scalar;
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
}

function sample(id: number, keys: string[], C: Float64Array, range: SampleRange, quality: Quality): Extract<FromWorker, { type: "result" }> {
  const curves: CurveResult[] = [];
  const warnings: WorkerWarning[] = [];
  const origin: [number, number] = [(range.x0 + range.x1) / 2, (range.y0 + range.y1) / 2];
  let total = 0;
  for (const key of keys) {
    const err = compileErrors.get(key);
    if (err) {
      warnings.push({ source: key, code: "W-codegen", message: `계산 코드를 만들 수 없음 (${err})` });
      continue;
    }
    const c = compiled.get(key);
    if (!c) continue;
    if (total >= SCENE_BUDGET) {
      warnings.push({ source: key, code: "W-budget", message: "장면 샘플 예산을 넘어 그리지 않음" });
      continue;
    }
    R.capped = false;
    let xs: Float64Array, ys: Float64Array, offsets: number[], truncated: boolean;
    let kind: CurveResult["kind"];
    if (c.discrete) {
      ({ xs, ys, truncated } = sampleDiscrete(c.batch, C, range));
      offsets = [0, xs.length];
      kind = "points";
    } else {
      ({ xs, ys, offsets, truncated } = sampleCurve(c.batch, c.scalar, C, range, quality));
      kind = "polyline";
    }
    if (truncated) warnings.push({ source: key, code: "W-budget", message: "점이 너무 많아 품질을 낮춤" });
    if (R.capped) warnings.push({ source: key, code: "W-sum-terms", message: "sum/prod 항이 10만 개를 넘는 점은 그리지 않음" });
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
    clipExpr = m.clip;
    setHighlights(hlSpecs);
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
