// Plot controller: view + worker + renderer.
// - Drag and wheel only change the transform; geometry is recomputed when the view
//   leaves the computed area (one extra screen on each side) or the zoom drifts.
// - Coarse while interacting, fine once interaction stops. Latest request wins.
// - A constant change recomputes only the functions that use it.

import type { CheckResult } from "../generated/CheckResult";
import type { FeaturePoints } from "../generated/FeaturePoints";
import type { Function as Fn } from "../generated/Function";
import type { Highlight } from "../generated/Highlight";
import type { Nav } from "../generated/Nav";
import type { Scene } from "../generated/Scene";
import type { SpanDim } from "../generated/SpanDim";
import type { State } from "../generated/State";
import type { ViewSpec } from "../generated/ViewSpec";
import type { ViewBox } from "../generated/ViewBox";
import type { Warning } from "../generated/Warning";
import { identTex, isVectorName } from "../math/latex";
import { setImageReadyHandler } from "../math/mathjax";
import { compileConst, compileOrder, specsOf, type FnSpec } from "../worker/codegen";
import { makeRuntime } from "../worker/numeric";
import type { Quality, SampleRange } from "../worker/sample";
import { CLIP_KEY, type CurveResult, type FillItem, type FillResult, type FromWorker, type HlSpec, type RasterResult, type ToWorker } from "../worker/worker";
import type { RasterRect } from "../worker/fill";
import { checkSpecsOf, constSets } from "../worker/verify";
import { maskPixels, resolveHighlights, sortedHighlights } from "./highlight";
import { POINT_FEATURE, POINT_HOLE, POINT_NORMAL, rgba, type Batch, type ClipMask, type LabelPlacement, type LabelRef, type Style } from "./primitives";
import { FEATURE_WORD, pointText } from "./featureText";
import { activeClip, sceneAxes, visibleAt } from "./scene";
import { Renderer } from "./renderer";
import { buildPolarSpace, buildSpace, spaceStyles } from "./space";
import { View } from "./view";

export const PALETTE = ["#6ea8ff", "#ffa057", "#c08cff", "#5fd38a"];
export const colorOf = (f: Fn) => PALETTE[f.color % PALETTE.length]!;

/** Recompute when the zoom changed by more than this factor since sampling. */
const ZOOM_DRIFT = 1.5;
const IDLE_MS = 180;
/** Raster area: the visible area plus this fraction of it on each side. */
const RASTER_MARGIN = 0.25;
/** Rasters are pixel images: recompute sooner when the zoom changes. */
const RASTER_DRIFT = 1.2;
const RASTER_MAX = 4096;
/** Boundary line opacity of a function fill (mockup: 0.55). */
const EDGE_ALPHA = 0.55;

interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

interface Curve {
  result: CurveResult;
  range: SampleRange;
  quality: Quality;
  /** `depStamp` when computed: the definition and everything it refers to. */
  version: string;
}

/** Fill opacity of a span drawn as a plane. */
const PLANE_ALPHA = 0.1;

interface HlGeom {
  result: FillResult | (RasterResult & { pixels: Uint8ClampedArray });
  range: SampleRange;
  /** Pixels per world unit when computed. */
  scale: number;
  quality: Quality;
  stamp: string;
}

export interface PlotCallbacks {
  /** All functions of `revision` are drawn at fine quality. */
  rendered(revision: number, warnings: Warning[]): void;
}

interface Cam {
  cx: number;
  cy: number;
  scale: number;
}

const CONST_R = makeRuntime();

/** Camera for a `view` spec at the given canvas size; null if a bound is not a number. */
export function fitView(spec: ViewSpec, width: number, height: number): Cam | null {
  const ev = (r: { lo: import("../generated/Expr").Expr; hi: import("../generated/Expr").Expr } | null) => {
    if (!r) return null;
    try {
      const a = compileConst(r.lo, CONST_R)(), b = compileConst(r.hi, CONST_R)();
      return Number.isFinite(a) && Number.isFinite(b) && b > a ? [a, b] as const : undefined;
    } catch {
      return undefined;
    }
  };
  const h = ev(spec.h), v = ev(spec.v);
  if (h === undefined || v === undefined || (!h && !v) || width <= 0 || height <= 0) return null;
  const sx = h ? width / (h[1] - h[0]) : Infinity, sy = v ? height / (v[1] - v[0]) : Infinity;
  return { cx: h ? (h[0] + h[1]) / 2 : 0, cy: v ? (v[0] + v[1]) / 2 : 0, scale: Math.min(sx, sy) };
}

function sameCam(a: Cam, b: Cam): boolean {
  return a.cx === b.cx && a.cy === b.cy && a.scale === b.scale;
}

export class Plot {
  readonly view = new View();
  private renderer: Renderer;
  private worker: Worker;
  private canvas: HTMLCanvasElement;
  private cursor: HTMLElement;

  private state: State | null = null;
  /** Functions of the current scene (sampled and drawn); `allFns` go to the worker. */
  private fns: Fn[] = [];
  private allFns: Fn[] = [];
  private members = new Set<string>();
  /** Functions and highlights not shown in the current scene and step. */
  private hidden = new Set<string>();
  private scene: Scene | null = null;
  private sceneKey: string | null = null;
  private step = 0;
  private axes: [string, string] = ["x", "y"];
  private axisTex: [string, string] = ["x", "y"];
  /** Camera per scene, kept when leaving (view applies on first entry only). */
  private cams = new Map<string, Cam>();
  /** Per scene: the `view` source last applied; a different one moves the camera again. */
  private appliedView = new Map<string, string>();
  /** Camera to set once the canvas has a size. */
  private pendingCam: (() => Cam | "reset") | null = null;
  /** Last camera fitted by a rule (step view), refitted on resize until the user moves it. */
  private fitted: { fit: () => Cam | "reset"; cam: Cam } | null = null;
  private clipKey = "";
  private clipGeom: { mask: ClipMask; bits: Uint8Array; scale: number; quality: Quality; stamp: string } | null = null;
  /** Scene waiting for its first-screen check, and the warnings it produced. */
  private entryCheck: string | null = null;
  private entryWarnings: Warning[] = [];
  private constKeys: string[] = [];
  private overrides = new Map<string, number>();

  private curves = new Map<string, Curve>();
  private hls: Highlight[] = [];
  private hlGeom = new Map<string, HlGeom>();
  private hlLatest = new Map<string, number>();
  private hlCoarseInFlight = false;
  /** Highlights a slider changed while a coarse fill was in flight: redone when it returns. */
  private hlPending = new Set<string>();
  private warningsByKey = new Map<string, Warning[]>();
  private reqId = 0;
  private latest = new Map<string, number>();
  private waitRendered: { id: number; revision: number } | null = null;
  private idleTimer = 0;
  private hlIdleTimer = 0;
  private coarseInFlight = false;
  /** Functions a slider changed while a coarse sample was in flight: redone when it returns. */
  private pending = new Set<string>();
  private drawQueued = false;
  /** Pointer position in CSS px while hovering (not dragging). */
  private hoverAt: { px: number; py: number } | null = null;
  /** Feature definitions whose coordinate labels the user turned off. */
  private coordOff = new Set<string>();
  /** Numeric verification (stage 5c): last results, the request in flight, its inputs. */
  private checkResults: CheckResult[] = [];
  private verifyId = 0;
  private verifyPending = false;
  private verifyStamp = "";
  /** A render to report once the checks in flight arrive (nothing else was resampled). */
  private checksWait: number | null = null;

  constructor(host: HTMLElement, private cb: PlotCallbacks) {
    this.canvas = document.createElement("canvas");
    this.canvas.className = "plot-canvas";
    host.append(this.canvas);
    this.cursor = host.querySelector(".cursor") as HTMLElement;
    this.renderer = new Renderer(this.canvas);
    this.worker = new Worker(new URL("../worker/worker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (e: MessageEvent<FromWorker>) => this.onResult(e.data);
    setImageReadyHandler(() => this.queueDraw());

    new ResizeObserver(() => {
      const r = host.getBoundingClientRect();
      this.view.resize(r.width, r.height);
      this.renderer.resize(r.width, r.height);
      if (this.pendingCam && r.width > 0) this.setCam(this.pendingCam);
      // The canvas changed size right after a fit (the step bar appearing): fit again.
      else if (this.fitted && sameCam(this.cam(), this.fitted.cam)) this.setCam(this.fitted.fit);
      this.draw();
      this.afterViewChange(false);
    }).observe(host);
    this.bindInput();
  }

  /** Shown (2D scene) or hidden (a 3D scene is drawn by `Plot3D`). */
  setActive(on: boolean): void {
    this.canvas.hidden = !on;
    if (!on) this.cursor.hidden = true;
  }

  // ----- model -----------------------------------------------------------

  /** New snapshot. Recomputes functions whose version or constants changed. */
  setState(state: State, revision: number, nav: Nav): void {
    const prev = this.state;
    this.state = state;
    const scene = (nav.scene && state.scenes[nav.scene]) || null;
    this.enterScene(scene);
    this.scene = scene;
    this.step = scene ? Math.min(nav.step, scene.steps.length - 1) : 0;
    [this.axes, this.axisTex] = sceneAxes(scene);
    this.allFns = Object.values(state.functions)
      .map((f) => f!)
      .sort((a, b) => a.order - b.order);
    this.members = new Set(scene?.members ?? []);
    // A calc is listed (panel) but has nothing to sample.
    this.fns = this.allFns.filter((f) => this.members.has(f.key) && f.kind !== "Calc");
    this.hidden = new Set([...Object.keys(state.functions), ...Object.keys(state.highlights)].filter((k) => !scene || !visibleAt(scene, this.step, k)));
    this.applyViewRule();
    this.constKeys = Object.keys(state.constants).sort();
    for (const k of [...this.curves.keys()]) if (!this.members.has(k)) this.curves.delete(k);
    for (const k of [...this.warningsByKey.keys()]) if (!state.functions[k]) this.warningsByKey.delete(k);

    const specs: FnSpec[] = specsOf(state, this.allFns);
    this.post({ type: "functions", fns: specs, constKeys: this.constKeys });
    // Posted before any sample: the worker answers in order, so the checks are in by the
    // time the render is reported.
    this.verify(state);
    const clip = activeClip(scene);
    const polar = scene?.polar ?? false;
    const clipKey = JSON.stringify([this.axes, polar, clip?.expr ?? null]);
    if (clipKey !== this.clipKey) {
      this.clipKey = clipKey;
      this.clipGeom = null;
      this.post({ type: "scene", axes: this.axes, polar, clip: clip?.expr ?? null });
    }

    const changedConsts = new Set(
      this.constKeys.filter((k) => !prev || prev.constants[k]?.version !== state.constants[k]!.version),
    );
    const dirty = this.fns
      .filter((f) => {
        const c = this.curves.get(f.key);
        return !c || c.version !== this.depStamp(f.key) || f.consts.some((k) => changedConsts.has(k));
      })
      .map((f) => f.key);
    this.setHighlights(state, changedConsts);
    if (clip && (!this.clipGeom || clip.consts.some((k) => changedConsts.has(k)) || this.clipGeom.stamp !== this.clipStamp())) this.requestHighlights([CLIP_KEY], "fine");
    if (dirty.length === 0) {
      this.draw();
      if (this.verifyPending) this.checksWait = revision;
      else this.cb.rendered(revision, this.warnings());
      this.checkEntry();
      return;
    }
    const id = this.request(dirty, "fine");
    this.waitRendered = { id, revision };
  }

  /**
   * Versions of a definition and of everything it refers to (transitively): a curve
   * that reads a changed vector or function is recomputed even though its own body
   * did not change.
   */
  private depStamp(key: string): string {
    const fns = this.state?.functions ?? {};
    const seen = new Set<string>();
    const stack = [key];
    while (stack.length) {
      const k = stack.pop()!;
      if (seen.has(k)) continue;
      seen.add(k);
      stack.push(...(fns[k]?.calls ?? []));
    }
    return [...seen].sort().map((k) => `${k}:${fns[k]?.version ?? -1}`).join(",");
  }

  /** Points each shown feature-point definition found at fine quality (`get_state`, apply). */
  featurePoints(): FeaturePoints[] {
    return this.fns
      .filter((f) => f.kind === "Feature" && !this.hidden.has(f.key))
      .flatMap((f) => {
        const c = this.curves.get(f.key);
        if (!c || c.quality !== "fine" || !c.result.points) return [];
        const points = c.result.points.map((p) => ({ x: p.vx, y: p.cls === "Asymptote" || !Number.isFinite(p.vy) ? null : p.vy, class: p.cls }));
        return [{ key: f.key, points, capped: !!c.result.capped }];
      });
  }

  /** Verification results of the current state (`apply`, `get_state`, panel). */
  checks(): CheckResult[] {
    return this.checkResults;
  }

  /**
   * Runs the checks the state implies when a compared definition or a constant changed:
   * at the current constants and two random sets inside the slider ranges.
   */
  private verify(state: State): void {
    const specs = checkSpecsOf(state);
    const stamp = JSON.stringify([specs.map((c) => [c.key, c.a, c.b, this.depStamp(c.a), this.depStamp(c.b), this.depStamp(c.key)]), this.constKeys.map((k) => state.constants[k]!.version)]);
    if (stamp === this.verifyStamp) return;
    this.verifyStamp = stamp;
    if (specs.length === 0) {
      this.checkResults = [];
      return;
    }
    const index = new Map(this.constKeys.map((k, i) => [k, i]));
    const orders = this.allFns.flatMap((f) => f.constraints).flatMap((c) => {
      try {
        return [compileOrder(c.lo, c.hi, index, CONST_R)];
      } catch {
        return [];
      }
    });
    const sets = constSets(state, this.constKeys, this.constants(), (C) => orders.every((o) => o(C)));
    this.verifyId++;
    this.verifyPending = true;
    this.post({ type: "verify", id: this.verifyId, specs, sets });
  }

  /** Last points of a feature definition (panel), or undefined before it is computed. */
  featureResult(key: string): CurveResult | undefined {
    const c = this.curves.get(key);
    return c?.result.kind === "features" ? c.result : undefined;
  }

  /** Coordinate labels beside feature points (on by default; per definition). */
  coordLabels(key: string): boolean {
    return !this.coordOff.has(key);
  }

  toggleCoordLabels(key: string): void {
    if (this.coordOff.has(key)) this.coordOff.delete(key);
    else this.coordOff.add(key);
    this.draw();
  }

  /** Dimension of each shown span as last drawn at fine quality (`get_state`, apply). */
  spanDims(): SpanDim[] {
    return this.fns
      .filter((f) => f.kind === "Span" && !this.hidden.has(f.key))
      .flatMap((f) => {
        const c = this.curves.get(f.key);
        return c && c.result.dim !== undefined ? [{ key: f.key, dim: c.result.dim }] : [];
      });
  }

  // ----- scene -----------------------------------------------------------

  private cam(): Cam {
    return { cx: this.view.cx, cy: this.view.cy, scale: this.view.scale };
  }

  private setCam(c: Cam | "reset" | (() => Cam | "reset")): void {
    if (this.view.width === 0) {
      this.pendingCam = typeof c === "function" ? c : () => c;
      return;
    }
    this.pendingCam = null;
    const cam = typeof c === "function" ? c() : c;
    if (cam === "reset") this.view.reset();
    else Object.assign(this.view, cam);
    this.fitted = typeof c === "function" ? { fit: c, cam: this.cam() } : null;
  }

  /** Camera for the current step's view (default range without one). */
  private stepCam(): Cam | "reset" {
    const spec = this.scene?.steps[this.step]?.view ?? null;
    return (spec && fitView(spec, this.view.width, this.view.height)) ?? "reset";
  }

  /** Scene switch: keep the old camera, restore the new scene's (first entry: its view). */
  private enterScene(scene: Scene | null): void {
    const key = scene?.key ?? null;
    if (key === this.sceneKey) return;
    if (this.sceneKey) this.cams.set(this.sceneKey, this.cam());
    this.sceneKey = key;
    this.entryWarnings = [];
    this.entryCheck = null;
    if (!key) return;
    const cam = this.cams.get(key);
    if (cam) {
      this.setCam(cam);
    } else {
      // First entry: the view is applied by `applyViewRule`; check the first screen.
      this.entryCheck = key;
    }
    this.curves.clear();
    this.hlGeom.clear();
  }

  /** The view of the current step differs from the one last applied: move the camera. */
  private applyViewRule(): void {
    if (!this.scene) return;
    const src = this.scene.steps[this.step]?.view?.src ?? "";
    if (this.appliedView.get(this.scene.key) === src) return;
    this.appliedView.set(this.scene.key, src);
    // Fitted now, or after the first resize.
    this.setCam(() => this.stepCam());
  }

  /** First screen of a scene: functions with no visible point; nothing visible inside a clip. */
  private checkEntry(): void {
    if (!this.entryCheck || this.entryCheck !== this.sceneKey || !this.state) return;
    const clip = activeClip(this.scene);
    if (clip && this.clipGeom?.quality !== "fine") return;
    const shown = this.fns.filter((f) => !this.hidden.has(f.key));
    if (shown.some((f) => this.curves.get(f.key)?.quality !== "fine")) return;
    this.entryCheck = null;
    const b = this.view.box();
    const out: Warning[] = [];
    let any = false;
    for (const f of shown) {
      const r = this.curves.get(f.key)!.result;
      // No feature point is a result (x⁴ has no inflection point), not a missing curve.
      if (r.kind === "features") continue;
      // A plane covers the whole computed area.
      let seen = r.kind === "span" && r.dim === 2 && (!clip || this.clipGeom!.bits.includes(1));
      for (let i = 0; i < r.coords.length && !seen; i += 2) {
        const x = r.origin[0] + r.coords[i]!, y = r.origin[1] + r.coords[i + 1]!;
        seen = x >= b.x_min && x <= b.x_max && y >= b.y_min && y <= b.y_max && this.inClip(x, y);
      }
      any ||= seen;
      if (!seen) out.push({ source: f.key, code: "W-offscreen", message: "처음 화면에 보이는 점 없음" });
    }
    if (clip && !any && shown.length > 0) out.push({ source: null, code: "W-empty-screen", message: "처음 화면이 비어 있음 (clip 범위 확인)" });
    this.entryWarnings = out;
    this.cb.rendered(-1, this.warnings());
  }

  /** Inside the scene clip (true without one, or before its mask arrives). */
  private inClip(x: number, y: number): boolean {
    const g = this.clipGeom;
    if (!activeClip(this.scene) || !g) return true;
    const r = g.mask.rect;
    if (x < r.x0 || x > r.x1 || y < r.y0 || y > r.y1) return false;
    const i = Math.min(g.mask.width - 1, Math.floor(((x - r.x0) / (r.x1 - r.x0)) * g.mask.width));
    const j = Math.min(g.mask.height - 1, Math.floor(((r.y1 - y) / (r.y1 - r.y0)) * g.mask.height));
    return g.bits[j * g.mask.width + i] === 1;
  }

  private clipStamp(): string {
    const clip = activeClip(this.scene);
    const fv = (k: string) => this.state?.functions[k]?.version ?? -1;
    return clip ? `${this.clipKey}|${Object.keys(this.state?.functions ?? {}).map(fv).join(",")}` : "";
  }

  /** Highlights drawn on the canvas (function fills and targetless regions) in this scene. */
  private drawable(): Highlight[] {
    return this.hls.filter((h) => h.target.type !== "Sub" && this.members.has(h.key));
  }

  /** Changes when anything the geometry depends on changed (except constant values). */
  private stamp(h: Highlight): string {
    const fv = (k: string) => this.state?.functions[k]?.version ?? -1;
    return `${h.version}|${h.deps.map(fv).join(",")}`;
  }

  private setHighlights(state: State, changedConsts: Set<string>): void {
    this.hls = sortedHighlights(state);
    for (const k of [...this.hlGeom.keys()]) if (!state.highlights[k]) this.hlGeom.delete(k);
    const specs: HlSpec[] = this.drawable().map((h) => ({ key: h.key, fn: h.target.type === "Function" ? h.target.function : null, range: h.range }));
    this.post({ type: "highlights", specs });
    const dirty = this.drawable().filter((h) => {
      const g = this.hlGeom.get(h.key);
      return !g || g.stamp !== this.stamp(h) || h.consts.some((k) => changedConsts.has(k));
    });
    this.requestHighlights(dirty.map((h) => h.key), "fine");
  }

  private rasterRect(): { rect: RasterRect; width: number; height: number } {
    const b = this.view.box();
    const mw = (b.x_max - b.x_min) * RASTER_MARGIN, mh = (b.y_max - b.y_min) * RASTER_MARGIN;
    const k = 1 + 2 * RASTER_MARGIN;
    return {
      rect: { x0: b.x_min - mw, x1: b.x_max + mw, y0: b.y_min - mh, y1: b.y_max + mh },
      width: Math.min(RASTER_MAX, Math.max(1, Math.round(this.view.width * k))),
      height: Math.min(RASTER_MAX, Math.max(1, Math.round(this.view.height * k))),
    };
  }

  private requestHighlights(keys: string[], quality: Quality): void {
    if (keys.length === 0 || !this.state) {
      if (quality === "coarse") this.hlCoarseInFlight = false;
      return;
    }
    const id = ++this.reqId;
    const coarse = quality === "coarse";
    if (!coarse) for (const k of keys) this.hlPending.delete(k);
    const items: FillItem[] = [];
    for (const k of keys) {
      if (k === CLIP_KEY) {
        if (!activeClip(this.scene)) continue;
        this.hlLatest.set(k, id);
        const r = this.rasterRect();
        const f = coarse ? 0.35 : 1;
        items.push({ key: k, kind: "raster", rect: r.rect, width: Math.max(1, Math.round(r.width * f)), height: Math.max(1, Math.round(r.height * f)) });
        continue;
      }
      const h = this.state.highlights[k];
      if (!h || h.target.type === "Sub") continue;
      this.hlLatest.set(k, id);
      if (h.target.type === "Grid") {
        const r = this.rasterRect();
        const f = coarse ? 0.35 : 1;
        items.push({ key: k, kind: "raster", rect: r.rect, width: Math.max(1, Math.round(r.width * f)), height: Math.max(1, Math.round(r.height * f)) });
      } else {
        const rule = h.style.fill === "Region" ? "Axis" : h.style.fill;
        items.push({ key: k, kind: "fill", rule, n: Math.max(64, Math.round(this.view.width * (coarse ? 1 : 3))) });
      }
    }
    this.post({ type: "fills", id, items, C: this.constants(), range: this.computeRange(), quality });
  }

  private onFills(r: Extract<FromWorker, { type: "fills" }>): void {
    if (r.quality === "coarse") this.hlCoarseInFlight = false;
    for (const res of r.results) {
      if (this.hlLatest.get(res.key) !== r.id) continue;
      if (res.key === CLIP_KEY && res.kind === "raster") {
        const mask: ClipMask = { id: CLIP_KEY, rect: res.rect, width: res.width, height: res.height, pixels: maskPixels(res.mask, "#ffffff", 1) };
        this.clipGeom = { mask, bits: res.mask, scale: this.view.scale, quality: r.quality, stamp: this.clipStamp() };
        continue;
      }
      const h = this.state?.highlights[res.key];
      if (!h) continue;
      const result = res.kind === "raster" ? { ...res, pixels: maskPixels(res.mask, h.style.color, h.style.opacity) } : res;
      this.hlGeom.set(res.key, { result, range: r.range, scale: this.view.scale, quality: r.quality, stamp: this.stamp(h) });
    }
    if (r.quality === "fine") {
      for (const key of new Set(r.results.map((x) => x.key))) this.warningsByKey.delete(key);
      for (const w of r.warnings) {
        const k = w.source ?? "";
        this.warningsByKey.set(k, [...(this.warningsByKey.get(k) ?? []), { source: w.source, code: w.code, message: w.message }]);
      }
    }
    this.draw();
    if (r.quality === "coarse") this.flushPendingHighlights();
    this.checkEntry();
  }

  /** Highlights whose geometry no longer covers the view (or is too coarse for the zoom). */
  private staleHighlights(): string[] {
    const b = this.view.box();
    const out: string[] = [];
    if (activeClip(this.scene)) {
      const g = this.clipGeom;
      const r = g?.mask.rect;
      const drift = g ? Math.max(this.view.scale / g.scale, g.scale / this.view.scale) : Infinity;
      if (!g || !r || b.x_min < r.x0 || b.x_max > r.x1 || b.y_min < r.y0 || b.y_max > r.y1 || drift > RASTER_DRIFT) out.push(CLIP_KEY);
    }
    return out.concat(this.drawable()
      .filter((h) => {
        const g = this.hlGeom.get(h.key);
        if (!g) return true;
        const drift = Math.max(this.view.scale / g.scale, g.scale / this.view.scale);
        if (g.result.kind === "raster") {
          const r = g.result.rect;
          return b.x_min < r.x0 || b.x_max > r.x1 || b.y_min < r.y0 || b.y_max > r.y1 || drift > RASTER_DRIFT;
        }
        const r = g.range;
        return b.x_min < r.x0 || b.x_max > r.x1 || b.y_min < r.y0 || b.y_max > r.y1 || drift > ZOOM_DRIFT;
      })
      .map((h) => h.key));
  }

  /** Slider: local value while dragging (`final` = released). */
  setConstant(key: string, value: number, final: boolean): void {
    this.overrides.set(key, value);
    const keys = this.fns.filter((f) => f.consts.includes(key)).map((f) => f.key);
    const hkeys = this.drawable().filter((h) => h.consts.includes(key)).map((h) => h.key);
    if (final) {
      this.request(keys, "fine");
      this.requestHighlights(hkeys, "fine");
    } else {
      // One coarse request at a time; values that arrive meanwhile are sent when it
      // returns, so the last value of a drag is always drawn.
      if (!this.coarseInFlight) {
        this.coarseInFlight = true;
        this.request(keys, "coarse");
      } else for (const k of keys) this.pending.add(k);
      if (!this.hlCoarseInFlight && hkeys.length > 0) {
        this.hlCoarseInFlight = true;
        this.requestHighlights(hkeys, "coarse");
      } else for (const k of hkeys) this.hlPending.add(k);
    }
  }

  /**
   * Split boundary order a slider value would break (`"a < b"`), or null. Constraints
   * that need more than constants to evaluate are left to the overlap warning.
   */
  orderViolation(key: string, value: number): string | null {
    if (!this.state) return null;
    const index = new Map(this.constKeys.map((k, i) => [k, i]));
    const C = this.constants();
    const i = index.get(key);
    if (i === undefined) return null;
    C[i] = value;
    for (const f of this.fns) {
      if (!f.consts.includes(key)) continue;
      for (const c of f.constraints) {
        let holds: boolean;
        try {
          holds = compileOrder(c.lo, c.hi, index, CONST_R)(C);
        } catch {
          continue;
        }
        if (!holds) return c.src;
      }
    }
    return null;
  }

  /** The server confirmed the final value; drop the local override. */
  clearOverride(key: string): void {
    this.overrides.delete(key);
  }

  /** Redraw what uses `key` at the state's value (after a local animation ended). */
  releaseConstant(key: string): void {
    this.overrides.delete(key);
    this.request(this.fns.filter((f) => f.consts.includes(key)).map((f) => f.key), "fine");
    this.requestHighlights(this.drawable().filter((h) => h.consts.includes(key)).map((h) => h.key), "fine");
  }

  warnings(): Warning[] {
    return [...[...this.warningsByKey.values()].flat(), ...this.entryWarnings];
  }

  warningsFor(key: string): Warning[] {
    return [...(this.warningsByKey.get(key) ?? []), ...this.entryWarnings.filter((w) => w.source === key)];
  }

  /** Scene-wide warnings (no definition). */
  sceneWarnings(): Warning[] {
    return this.entryWarnings.filter((w) => w.source === null);
  }

  /** Current axis names, for the cursor and `get_state`. */
  axisNames(): [string, string] {
    return this.axes;
  }

  box(): ViewBox {
    return this.view.box();
  }

  /** Canvas as PNG (base64), long side at most `maxSide` px. Labels are part of the canvas. */
  png(maxSide: number): { data: string; width: number; height: number } {
    this.draw();
    const src = this.canvas;
    const k = Math.min(1, maxSide / Math.max(src.width, src.height));
    const out = document.createElement("canvas");
    out.width = Math.max(1, Math.round(src.width * k));
    out.height = Math.max(1, Math.round(src.height * k));
    const ctx = out.getContext("2d")!;
    // The card background is CSS; paint it so the PNG is not transparent.
    ctx.fillStyle = "#212329";
    ctx.fillRect(0, 0, out.width, out.height);
    ctx.drawImage(src, 0, 0, out.width, out.height);
    const url = out.toDataURL("image/png");
    return { data: url.slice(url.indexOf(",") + 1), width: out.width, height: out.height };
  }

  /** Back to the current step's view (or the default range). */
  resetView(): void {
    this.setCam(this.stepCam());
    this.draw();
    this.afterViewChange(true);
  }

  zoom(factor: number): void {
    this.view.zoomAt(this.view.width / 2, this.view.height / 2, factor);
    this.draw();
    this.afterViewChange(true);
  }

  // ----- sampling --------------------------------------------------------

  private post(m: ToWorker): void {
    this.worker.postMessage(m);
  }

  private constants(): Float64Array {
    return Float64Array.from(this.constKeys.map((k) => this.overrides.get(k) ?? this.state!.constants[k]!.value));
  }

  private computeRange(): SampleRange {
    const b = this.view.box();
    const w = b.x_max - b.x_min, h = b.y_max - b.y_min;
    return { x0: b.x_min - w, x1: b.x_max + w, y0: b.y_min - h, y1: b.y_max + h, sx: this.view.scale, sy: this.view.scale };
  }

  private request(keys: string[], quality: Quality): number {
    const id = ++this.reqId;
    if (keys.length === 0 || !this.state) {
      if (quality === "coarse") this.coarseInFlight = false;
      return id;
    }
    for (const k of keys) this.latest.set(k, id);
    if (quality === "fine") for (const k of keys) this.pending.delete(k);
    // A full fine request supersedes a pending render wait.
    if (quality === "fine" && this.waitRendered && keys.length === this.fns.length) this.waitRendered.id = id;
    this.post({ type: "sample", id, keys, C: this.constants(), range: this.computeRange(), quality });
    return id;
  }

  private onResult(r: FromWorker): void {
    if (r.type === "fills") return this.onFills(r);
    // 3D scenes are drawn by `Plot3D` with its own worker.
    if (r.type === "result3d") return;
    if (r.type === "checks") {
      if (r.id !== this.verifyId) return;
      this.checkResults = r.results;
      this.verifyPending = false;
      const rev = this.checksWait;
      this.checksWait = null;
      // -1 refreshes the panel only.
      this.cb.rendered(rev ?? -1, this.warnings());
      return;
    }
    if (r.quality === "coarse") this.coarseInFlight = false;
    let dimChanged = false;
    for (const c of r.curves) {
      if (this.latest.get(c.key) !== r.id) continue;
      const f = this.state?.functions[c.key];
      if (!f) continue;
      if (c.kind === "span" && this.curves.get(c.key)?.result.dim !== c.dim) dimChanged = true;
      // The panel lists feature points: refresh it when they change (slider released).
      if (c.kind === "features" && r.quality === "fine" && JSON.stringify(this.curves.get(c.key)?.result.points) !== JSON.stringify(c.points)) dimChanged = true;
      this.curves.set(c.key, { result: c, range: r.range, quality: r.quality, version: this.depStamp(f.key) });
    }
    if (r.quality === "fine") {
      for (const key of new Set(r.curves.map((c) => c.key))) this.warningsByKey.delete(key);
      for (const w of r.warnings) {
        const k = w.source ?? "";
        this.warningsByKey.set(k, [...(this.warningsByKey.get(k) ?? []), { source: w.source, code: w.code, message: w.message }]);
      }
    }
    this.draw();
    if (r.quality === "coarse") this.flushPending();
    if (this.waitRendered && r.id >= this.waitRendered.id && r.quality === "fine") {
      const rev = this.waitRendered.revision;
      this.waitRendered = null;
      this.cb.rendered(rev, this.warnings());
    }
    if (r.quality === "fine") this.checkEntry();
    // A slider turned a span into a line / plane: the panel shows the new dimension
    // right away, also while dragging.
    if (dimChanged) this.cb.rendered(-1, this.warnings());
  }

  private flushPending(): void {
    if (this.pending.size === 0) return;
    const keys = [...this.pending];
    this.pending.clear();
    this.coarseInFlight = true;
    this.request(keys, "coarse");
  }

  private flushPendingHighlights(): void {
    if (this.hlPending.size === 0) return;
    const keys = [...this.hlPending];
    this.hlPending.clear();
    this.hlCoarseInFlight = true;
    this.requestHighlights(keys, "coarse");
  }

  /** After pan/zoom: resample if the view left the computed area or the zoom drifted. */
  private afterViewChange(interacting: boolean): void {
    if (!this.state) return;
    // The first-screen check is about the first screen only.
    if (interacting && this.entryWarnings.length > 0 && !this.entryCheck) this.entryWarnings = [];
    const staleHl = this.staleHighlights();
    if (staleHl.length > 0 && interacting && !this.hlCoarseInFlight) {
      this.hlCoarseInFlight = true;
      this.requestHighlights(staleHl, "coarse");
    }
    if (staleHl.length > 0 || this.drawable().some((h) => this.hlGeom.get(h.key)?.quality === "coarse")) {
      clearTimeout(this.hlIdleTimer);
      this.hlIdleTimer = window.setTimeout(() => {
        const need = new Set(this.staleHighlights());
        for (const h of this.drawable()) if (this.hlGeom.get(h.key)?.quality === "coarse") need.add(h.key);
        this.requestHighlights([...need], "fine");
      }, interacting ? IDLE_MS : 0);
    }
    if (this.fns.length === 0) return;
    const b = this.view.box();
    const stale = this.fns.some((f) => {
      const c = this.curves.get(f.key);
      if (!c) return true;
      const r = c.range;
      const drift = Math.max(this.view.scale / r.sx, r.sx / this.view.scale);
      return b.x_min < r.x0 || b.x_max > r.x1 || b.y_min < r.y0 || b.y_max > r.y1 || drift > ZOOM_DRIFT;
    });
    const all = this.fns.map((f) => f.key);
    if (stale && interacting && !this.coarseInFlight) {
      this.coarseInFlight = true;
      this.request(all, "coarse");
    }
    clearTimeout(this.idleTimer);
    this.idleTimer = window.setTimeout(() => {
      const needFine = this.fns.some((f) => {
        const c = this.curves.get(f.key);
        if (!c || c.quality === "coarse") return true;
        const r = c.range;
        const bb = this.view.box();
        return bb.x_min < r.x0 || bb.x_max > r.x1 || bb.y_min < r.y0 || bb.y_max > r.y1 || Math.max(this.view.scale / r.sx, r.sx / this.view.scale) > ZOOM_DRIFT;
      });
      if (needFine) this.request(all, "fine");
    }, interacting ? IDLE_MS : 0);
  }

  // ----- drawing ---------------------------------------------------------

  private queueDraw(): void {
    if (this.drawQueued) return;
    this.drawQueued = true;
    requestAnimationFrame(() => {
      this.drawQueued = false;
      this.draw();
    });
  }

  private draw(): void {
    if (this.view.width === 0) return;
    const styles: Style[] = [...spaceStyles];
    const batches: Batch[] = this.scene?.polar ? buildPolarSpace(this.view, this.axisTex) : buildSpace(this.view, this.axisTex);
    this.drawHighlights(batches, styles);
    let seq = 0;
    const placed: Rect[] = [];
    for (const f of this.fns) {
      const c = this.curves.get(f.key);
      if (!c || this.hidden.has(f.key)) continue;
      const color = colorOf(f);
      const style = styles.length;
      styles.push({ color: rgba(color), width: 2.5, size: 7 });
      const r = c.result;
      const base = { id: f.key, version: f.version, source: { def: f.key }, seq: seq++, style, origin: [r.origin[0], r.origin[1], 0] as [number, number, number], dim: 2 as const, clip: true };
      if (r.kind === "features") {
        this.drawFeatures(f, r, base, styles, batches, color, placed);
        continue;
      }
      if (r.kind === "points" || (r.kind === "span" && r.dim === 0)) {
        if (r.kind === "span") styles[style]!.size = 10;
        batches.push({ ...base, layer: 3, kind: "points", coords: r.coords, pointKind: new Uint8Array(r.coords.length / 2).fill(POINT_NORMAL) });
      } else if (r.kind === "span" && r.dim === 2) {
        styles[style] = { color: rgba(color, PLANE_ALPHA), width: 0, size: 0 };
        batches.push({ ...base, layer: 1, kind: "polygon", coords: r.coords, ringOffsets: Uint32Array.of(0, 4), polyOffsets: Uint32Array.of(0, 1) });
      } else if (r.kind === "arrow") {
        styles[style] = { color: rgba(color), width: 2.5, size: 14 };
        batches.push({ ...base, layer: 3, kind: "arrows", coords: r.coords });
      } else {
        batches.push({ ...base, layer: 2, kind: "polyline", coords: r.coords, offsets: r.offsets });
      }
      // A plane fills the screen: its name is in the panel only.
      const at = r.kind === "span" && r.dim === 2 ? null : r.kind === "arrow" ? this.arrowLabel(r) : this.placeLabel(r, f.key.length, placed);
      if (at) {
        batches.push({
          id: `label-${f.key}`,
          version: f.version,
          source: { def: f.key },
          layer: 4,
          seq: 100 + seq,
          style,
          origin: [0, 0, 0],
          dim: 2,
          kind: "text",
          coords: Float32Array.from([at.x, at.y]),
          labels: [{ kind: "tex", tex: identTex(f.name, undefined, isVectorName(f)), px: 22 }],
          placement: [at.place],
        });
      }
    }
    const hit = this.hoverAt ? this.nearestPoint(this.hoverAt.px, this.hoverAt.py) : null;
    this.canvas.classList.toggle("snap", !!hit);
    if (hit) {
      const style = styles.length;
      styles.push({ color: rgba(hit.color), width: 2.5, size: 11 });
      const label = styles.length;
      styles.push({ color: rgba("#ecedf0"), width: 0, size: 0 });
      batches.push({ id: "hover-point", version: 0, source: { def: hit.key }, layer: 3, seq: 1000, style, origin: [0, 0, 0], dim: 2, kind: "points", coords: Float32Array.from([hit.x, hit.y]), pointKind: Uint8Array.of(POINT_HOLE) });
      const d = this.decimals();
      const fmt = (v: number) => v.toFixed(d).replace("-", "\u2212");
      const text = hit.text ?? `${hit.key}  (${fmt(hit.x)}, ${fmt(hit.y)})`;
      const right = this.view.sx(hit.x) < this.view.width - 160;
      batches.push({
        id: "hover-label",
        version: 0,
        source: { def: hit.key },
        layer: 4,
        seq: 1001,
        style: label,
        origin: [0, 0, 0],
        dim: 2,
        kind: "text",
        coords: Float32Array.from([hit.x, hit.y]),
        labels: [{ kind: "ui", text, px: 13, weight: 600 }],
        placement: [{ align: right ? "left" : "right", baseline: "bottom", dx: right ? 12 : -12, dy: -10, halo: true }],
      });
    }
    this.renderer.draw(this.view, batches, styles, activeClip(this.scene) && this.clipGeom ? this.clipGeom.mask : null);
  }

  /**
   * Feature points: open circles where the function does not take the value (holes,
   * jump sides), filled dots for values it takes, ringed dots for the features themselves;
   * vertical asymptotes as dashed lines. Coordinates beside each point while few are on screen.
   */
  private drawFeatures(f: Fn, r: CurveResult, base: Omit<Extract<Batch, { kind: "points" }>, "kind" | "coords" | "pointKind" | "layer">, styles: Style[], batches: Batch[], color: string, placed: Rect[]): void {
    const pts = r.points ?? [];
    const [ox, oy] = r.origin;
    const b = this.view.box();
    const marks = pts.filter((p) => p.cls !== "Asymptote" && Number.isFinite(p.y));
    if (marks.length) {
      styles[base.style]!.size = 10;
      const coords = new Float32Array(marks.length * 2);
      const kinds = new Uint8Array(marks.length);
      marks.forEach((p, i) => {
        coords[2 * i] = p.x - ox;
        coords[2 * i + 1] = p.y - oy;
        kinds[i] = p.open ? POINT_HOLE : p.cls === "Filled" || p.cls === "Jump" || p.cls === "Endpoint" ? POINT_NORMAL : POINT_FEATURE;
      });
      batches.push({ ...base, layer: 3, kind: "points", coords, pointKind: kinds });
    }
    const asym = pts.filter((p) => p.cls === "Asymptote");
    if (asym.length) {
      const st = styles.length;
      styles.push({ color: rgba(color, 0.85), width: 1.5, size: 0, dash: [6, 5] });
      const h = b.y_max - b.y_min;
      const coords = new Float32Array(asym.length * 4);
      asym.forEach((p, i) => coords.set([p.x - ox, b.y_min - h - oy, p.x - ox, b.y_max + h - oy], 4 * i));
      batches.push({ ...base, id: `${f.key}-asym`, style: st, layer: 2, kind: "polyline", coords, offsets: Uint32Array.from({ length: asym.length + 1 }, (_, i) => 2 * i) });
    }
    const visible = pts.filter((p) => p.x >= b.x_min && p.x <= b.x_max && (!Number.isFinite(p.y) || (p.y >= b.y_min && p.y <= b.y_max)) && this.inClip(p.x, Number.isFinite(p.y) ? p.y : (b.y_min + b.y_max) / 2));
    // Where a point is drawn, or the top of the screen for an asymptote.
    const anchor = (p: (typeof pts)[number]) => [p.x, Number.isFinite(p.y) ? p.y : b.y_max - 24 / this.view.scale] as const;
    const axis = f.feature?.axes[0]?.key.replace(/^\\/, "") ?? "x";
    if (this.coordLabels(f.key) && visible.length > 0 && visible.length <= 12) {
      const st = styles.length;
      styles.push({ color: rgba("#c9ccd3"), width: 0, size: 0 });
      const coords = new Float32Array(visible.length * 2);
      const labels: LabelRef[] = [];
      const placement: LabelPlacement[] = [];
      visible.forEach((p, i) => {
        const [x, y] = anchor(p);
        coords.set([x, y], 2 * i);
        labels.push({ kind: "ui", text: pointText(p, axis), px: 12, weight: 500 });
        placement.push({ align: "left", baseline: "top", dx: 9, dy: 6, halo: true });
      });
      batches.push({ id: `coords-${f.key}`, version: f.version, source: { def: f.key }, layer: 4, seq: 200 + base.seq, style: st, origin: [0, 0, 0], dim: 2, kind: "text", coords, labels, placement });
    }
    const first = visible[0];
    if (first) {
      const [x, y] = anchor(first);
      const w = 14 * f.key.length + 10, px = this.view.sx(x), py = this.view.sy(y);
      placed.push({ left: px - w - 8, top: py - 34, right: px - 8, bottom: py - 8 });
      batches.push({
        id: `label-${f.key}`,
        version: f.version,
        source: { def: f.key },
        layer: 4,
        seq: 100 + base.seq,
        style: base.style,
        origin: [0, 0, 0],
        dim: 2,
        kind: "text",
        coords: Float32Array.from([x, y]),
        labels: [{ kind: "tex", tex: identTex(f.name), px: 20 }],
        placement: [{ align: "right", baseline: "bottom", dx: -8, dy: -8, halo: true }],
      });
    }
  }

  private drawHighlights(batches: Batch[], styles: Style[]): void {
    if (!this.state) return;
    const { fills } = resolveHighlights(this.state, this.hidden);
    let seq = 0;
    for (const h of fills) {
      const g = this.hlGeom.get(h.key);
      if (!g) continue;
      const r = g.result;
      const source = { def: h.key };
      if (r.kind === "raster") {
        batches.push({ id: `hl-${h.key}`, version: h.version, source, layer: 1, seq: seq++, style: 0, origin: [r.rect.x0, r.rect.y1, 0], dim: 2, kind: "image", size: [r.rect.x1 - r.rect.x0, r.rect.y1 - r.rect.y0], width: r.width, height: r.height, pixels: r.pixels, clip: true });
        continue;
      }
      const fill = styles.length;
      styles.push({ color: rgba(h.style.color, h.style.opacity), width: 0, size: 0 });
      const origin: [number, number, number] = [r.origin[0], r.origin[1], 0];
      const rings = r.rings.length - 1;
      batches.push({ id: `hl-${h.key}`, version: h.version, source, layer: 1, seq: seq++, style: fill, origin, dim: 2, kind: "polygon", coords: r.coords, ringOffsets: r.rings, polyOffsets: Uint32Array.from({ length: rings + 1 }, (_, i) => i), clip: true });
      if (h.style.width > 0) {
        const edge = styles.length;
        styles.push({ color: rgba(h.style.color, EDGE_ALPHA), width: h.style.width, size: 0 });
        batches.push({ id: `hl-edge-${h.key}`, version: h.version, source, layer: 1, seq: seq++, style: edge, origin, dim: 2, kind: "polyline", coords: r.edge, offsets: r.edgeOffsets, clip: true });
      }
    }
  }

  /** Coordinate display precision for the current zoom. */
  private decimals(): number {
    return Math.max(0, Math.min(10, Math.ceil(Math.log10(this.view.scale)) + 1));
  }

  /**
   * Closest point of a visible curve to the pointer, within a few pixels. On polylines
   * the point is interpolated between samples at the projection of the pointer.
   */
  private nearestPoint(px: number, py: number): { key: string; color: string; x: number; y: number; text?: string } | null {
    const v = this.view;
    const RADIUS = 12;
    let best: { key: string; color: string; x: number; y: number; text?: string } | null = null;
    let bestD = RADIUS * RADIUS;
    for (const f of this.fns) {
      const c = this.curves.get(f.key);
      if (!c || this.hidden.has(f.key)) continue;
      const r = c.result;
      if (r.kind === "span" && r.dim === 2) continue;
      if (r.kind === "features") {
        const axis = f.feature?.axes[0]?.key.replace(/^\\/, "") ?? "x";
        for (const p of r.points ?? []) {
          if (!Number.isFinite(p.y)) continue;
          const dx = v.sx(p.x) - px, dy = v.sy(p.y) - py, d = dx * dx + dy * dy;
          // Feature points win over a curve through the same spot.
          if (d < RADIUS * RADIUS && this.inClip(p.x, p.y) && (!best?.text || d < bestD)) {
            bestD = Math.min(bestD, d);
            best = { key: f.key, color: colorOf(f), x: p.x, y: p.y, text: `${f.key} ${FEATURE_WORD[p.cls]}  ${pointText(p, axis, this.decimals())}` };
          }
        }
        continue;
      }
      const [ox, oy] = r.origin;
      const X = (i: number) => ox + r.coords[2 * i]!, Y = (i: number) => oy + r.coords[2 * i + 1]!;
      const consider = (x: number, y: number) => {
        const dx = v.sx(x) - px, dy = v.sy(y) - py, d = dx * dx + dy * dy;
        if (d < bestD && !best?.text && this.inClip(x, y)) {
          bestD = d;
          best = { key: f.key, color: colorOf(f), x, y };
        }
      };
      if (r.kind === "points" || (r.kind === "span" && r.dim === 0)) {
        for (let i = 0; i < r.coords.length / 2; i++) consider(X(i), Y(i));
        continue;
      }
      for (let p = 0; p + 1 < r.offsets.length; p++) {
        for (let i = r.offsets[p]!; i + 1 < r.offsets[p + 1]!; i++) {
          const ax = v.sx(X(i)), ay = v.sy(Y(i)), bx = v.sx(X(i + 1)), by = v.sy(Y(i + 1));
          // Skip segments far from the pointer.
          if (Math.min(ax, bx) > px + RADIUS || Math.max(ax, bx) < px - RADIUS) continue;
          const sx = bx - ax, sy = by - ay, len2 = sx * sx + sy * sy;
          const t = len2 > 0 ? Math.min(1, Math.max(0, ((px - ax) * sx + (py - ay) * sy) / len2)) : 0;
          consider(X(i) + t * (X(i + 1) - X(i)), Y(i) + t * (Y(i + 1) - Y(i)));
        }
      }
    }
    return best;
  }

  /** Name label just past an arrow's head, on the side it points to. */
  private arrowLabel(c: CurveResult): { x: number; y: number; place: LabelPlacement } | null {
    const v = this.view;
    const [ox, oy] = c.origin;
    const hx = ox + c.coords[2]!, hy = oy + c.coords[3]!;
    const dx = v.sx(hx) - v.sx(ox + c.coords[0]!), dy = v.sy(hy) - v.sy(oy + c.coords[1]!);
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx / len, uy = dy / len;
    const place: LabelPlacement = {
      align: ux > 0.3 ? "left" : ux < -0.3 ? "right" : "center",
      baseline: uy > 0.3 ? "top" : uy < -0.3 ? "bottom" : "middle",
      dx: ux * 8,
      dy: uy * 8,
      halo: true,
    };
    return { x: hx, y: hy, place };
  }

  /**
   * Name label beside the curve: on the normal side away from the line, trying a few
   * positions from the right so labels of different curves do not overlap.
   */
  private placeLabel(c: CurveResult, chars: number, placed: Rect[]): { x: number; y: number; place: LabelPlacement } | null {
    const v = this.view;
    const [ox, oy] = c.origin;
    const n = c.coords.length / 2;
    if (n === 0) return null;
    const sx = (i: number) => v.sx(ox + c.coords[2 * i]!);
    const sy = (i: number) => v.sy(oy + c.coords[2 * i + 1]!);
    const inside = (px: number, py: number) => px > 40 && px < v.width - 70 && py > 30 && py < v.height - 40 && this.inClip(v.wx(px), v.wy(py));
    const w = 14 * chars + 10, hgt = 26, gap = 10;
    let fallback: { x: number; y: number; place: LabelPlacement } | null = null;
    for (const frac of [0.85, 0.7, 0.93, 0.55, 0.4, 0.25, 0.12]) {
      const target = v.width * frac;
      let best = -1, bestD = Infinity;
      for (let i = 0; i < n; i++) {
        const px = sx(i), py = sy(i);
        if (!inside(px, py)) continue;
        const d = Math.abs(px - target);
        if (d < bestD) {
          bestD = d;
          best = i;
        }
      }
      if (best < 0 || bestD > v.width * 0.1) continue;
      const a = Math.max(0, best - 1), b = Math.min(n - 1, best + 1);
      const tx = sx(b) - sx(a), ty = sy(b) - sy(a);
      const len = Math.hypot(tx, ty) || 1;
      // Normal pointing up for flat curves, to the right for steep ones.
      let nx = -ty / len, ny = tx / len;
      if (Math.abs(tx) >= Math.abs(ty) ? ny > 0 : nx < 0) {
        nx = -nx;
        ny = -ny;
      }
      const px = sx(best) + nx * gap, py = sy(best) + ny * gap;
      const place: LabelPlacement = { align: nx >= 0 ? "left" : "right", baseline: ny < 0 ? "bottom" : "top", dx: 0, dy: 0, halo: true };
      const left = place.align === "left" ? px : px - w;
      const top = place.baseline === "bottom" ? py - hgt : py;
      const rect: Rect = { left, top, right: left + w, bottom: top + hgt };
      const res = { x: v.wx(px), y: v.wy(py), place };
      fallback ??= res;
      if (rect.left < 4 || rect.right > v.width - 60 || rect.top < 4 || rect.bottom > v.height - 4) continue;
      if (placed.some((r) => r.left < rect.right && rect.left < r.right && r.top < rect.bottom && rect.top < r.bottom)) continue;
      placed.push(rect);
      return res;
    }
    return fallback;
  }

  // ----- input -----------------------------------------------------------

  private bindInput(): void {
    const el = this.canvas;
    let drag: { x: number; y: number } | null = null;
    el.addEventListener("pointerdown", (e) => {
      // Left button pans; the others do nothing here.
      if (e.button !== 0) return;
      drag = { x: e.clientX, y: e.clientY };
      el.setPointerCapture(e.pointerId);
      el.classList.add("dragging");
    });
    el.addEventListener("pointermove", (e) => {
      const r = el.getBoundingClientRect();
      this.showCursor(e.clientX - r.left, e.clientY - r.top);
      if (!drag) {
        this.hoverAt = { px: e.clientX - r.left, py: e.clientY - r.top };
        this.queueDraw();
        return;
      }
      this.hoverAt = null;
      this.view.panBy(e.clientX - drag.x, e.clientY - drag.y);
      drag = { x: e.clientX, y: e.clientY };
      this.queueDraw();
      this.afterViewChange(true);
    });
    const end = () => {
      if (!drag) return;
      drag = null;
      el.classList.remove("dragging");
      this.afterViewChange(true);
    };
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
    el.addEventListener("pointerleave", () => {
      this.cursor.hidden = true;
      this.hoverAt = null;
      this.queueDraw();
    });
    el.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        const r = el.getBoundingClientRect();
        this.view.zoomAt(e.clientX - r.left, e.clientY - r.top, Math.exp(-e.deltaY * 0.0015));
        this.queueDraw();
        this.afterViewChange(true);
      },
      { passive: false },
    );
  }

  private showCursor(px: number, py: number): void {
    const d = this.decimals();
    const fmt = (v: number) => v.toFixed(d).replace("-", "−");
    this.cursor.hidden = false;
    const [h, v] = this.axes.map((a) => a.replace(/^\\/, ""));
    const x = this.view.wx(px), y = this.view.wy(py);
    if (this.scene?.polar) {
      // Polar scenes read out the radius and the angle (0 ≤ θ < 2π).
      const t = Math.atan2(y, x);
      this.cursor.textContent = `${h} ${fmt(Math.hypot(x, y))} · ${v} ${fmt(t < 0 ? t + 2 * Math.PI : t)}`;
      return;
    }
    this.cursor.textContent = `${h} ${fmt(x)} · ${v} ${fmt(y)}`;
  }
}
