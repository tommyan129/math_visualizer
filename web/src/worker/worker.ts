// Evaluation worker: compiles functions, samples them, returns geometry as
// Transferable typed arrays. Layer, order and style are attached on the main thread.

import { CodegenError, compile, type Compiled, type FnSpec, type Scalar } from "./codegen";
import { makeRuntime } from "./numeric";
import { sampleCurve, sampleDiscrete, type Quality, type SampleRange } from "./sample";

export type ToWorker =
  | { type: "functions"; fns: FnSpec[]; constKeys: string[] }
  | { type: "sample"; id: number; keys: string[]; C: Float64Array; range: SampleRange; quality: Quality };

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

export type FromWorker = {
  type: "result";
  id: number;
  quality: Quality;
  range: SampleRange;
  curves: CurveResult[];
  warnings: WorkerWarning[];
};

/** Scene budget for 2D curve samples (design.md "2층 장면 예산"). */
const SCENE_BUDGET = 1_000_000;

const R = makeRuntime();
const F: Scalar[] = [];
let compiled = new Map<string, Compiled>();
let compileErrors = new Map<string, string>();
let constIndex = new Map<string, number>();

function setFunctions(fns: FnSpec[], constKeys: string[]): void {
  constIndex = new Map(constKeys.map((k, i) => [k, i]));
  const fnIndex = new Map(fns.map((f, i) => [f.key, i]));
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

function sample(id: number, keys: string[], C: Float64Array, range: SampleRange, quality: Quality): FromWorker {
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
  } else {
    const r = sample(m.id, m.keys, m.C, m.range, m.quality);
    const transfer = r.curves.flatMap((c) => [c.coords.buffer, c.offsets.buffer]);
    (self as unknown as Worker).postMessage(r, transfer);
  }
};
