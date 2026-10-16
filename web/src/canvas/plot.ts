// Plot controller: view + worker + renderer.
// - Drag and wheel only change the transform; geometry is recomputed when the view
//   leaves the computed area (one extra screen on each side) or the zoom drifts.
// - Coarse while interacting, fine once interaction stops. Latest request wins.
// - A constant change recomputes only the functions that use it.

import type { Function as Fn } from "../generated/Function";
import type { State } from "../generated/State";
import type { ViewBox } from "../generated/ViewBox";
import type { Warning } from "../generated/Warning";
import { identTex } from "../math/latex";
import { setImageReadyHandler } from "../math/mathjax";
import type { FnSpec } from "../worker/codegen";
import type { Quality, SampleRange } from "../worker/sample";
import type { CurveResult, FromWorker, ToWorker } from "../worker/worker";
import { POINT_HOLE, POINT_NORMAL, rgba, type Batch, type LabelPlacement, type Style } from "./primitives";
import { Renderer } from "./renderer";
import { buildSpace, spaceStyles } from "./space";
import { View } from "./view";

export const PALETTE = ["#6ea8ff", "#ffa057", "#c08cff", "#5fd38a"];
export const colorOf = (f: Fn) => PALETTE[f.color % PALETTE.length]!;

/** Recompute when the zoom changed by more than this factor since sampling. */
const ZOOM_DRIFT = 1.5;
const IDLE_MS = 180;

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
  version: number;
}

export interface PlotCallbacks {
  /** All functions of `revision` are drawn at fine quality. */
  rendered(revision: number, warnings: Warning[]): void;
}

export class Plot {
  readonly view = new View();
  private renderer: Renderer;
  private worker: Worker;
  private canvas: HTMLCanvasElement;
  private cursor: HTMLElement;

  private state: State | null = null;
  private fns: Fn[] = [];
  private hidden = new Set<string>();
  private constKeys: string[] = [];
  private overrides = new Map<string, number>();

  private curves = new Map<string, Curve>();
  private warningsByKey = new Map<string, Warning[]>();
  private reqId = 0;
  private latest = new Map<string, number>();
  private waitRendered: { id: number; revision: number } | null = null;
  private idleTimer = 0;
  private coarseInFlight = false;
  private drawQueued = false;
  /** Pointer position in CSS px while hovering (not dragging). */
  private hoverAt: { px: number; py: number } | null = null;

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
      this.draw();
      this.afterViewChange(false);
    }).observe(host);
    this.bindInput();
  }

  // ----- model -----------------------------------------------------------

  /** New snapshot. Recomputes functions whose version or constants changed. */
  setState(state: State, revision: number): void {
    const prev = this.state;
    this.state = state;
    this.fns = Object.values(state.functions)
      .map((f) => f!)
      .sort((a, b) => a.order - b.order);
    this.hidden = new Set(Object.values(state.scenes).flatMap((s) => s!.hidden));
    this.constKeys = Object.keys(state.constants).sort();
    for (const k of [...this.curves.keys()]) if (!state.functions[k]) this.curves.delete(k);
    for (const k of [...this.warningsByKey.keys()]) if (!state.functions[k]) this.warningsByKey.delete(k);

    const specs: FnSpec[] = this.fns.map((f) => ({ key: f.key, version: f.version, body: f.body, consts: f.consts, calls: f.calls }));
    this.post({ type: "functions", fns: specs, constKeys: this.constKeys });

    const changedConsts = new Set(
      this.constKeys.filter((k) => !prev || prev.constants[k]?.version !== state.constants[k]!.version),
    );
    const dirty = this.fns
      .filter((f) => {
        const c = this.curves.get(f.key);
        return !c || c.version !== f.version || f.consts.some((k) => changedConsts.has(k));
      })
      .map((f) => f.key);
    if (dirty.length === 0) {
      this.draw();
      this.cb.rendered(revision, this.warnings());
      return;
    }
    const id = this.request(dirty, "fine");
    this.waitRendered = { id, revision };
  }

  /** Slider: local value while dragging (`final` = released). */
  setConstant(key: string, value: number, final: boolean): void {
    this.overrides.set(key, value);
    const keys = this.fns.filter((f) => f.consts.includes(key)).map((f) => f.key);
    if (final) {
      this.request(keys, "fine");
    } else if (!this.coarseInFlight) {
      this.coarseInFlight = true;
      this.request(keys, "coarse");
    }
  }

  /** The server confirmed the final value; drop the local override. */
  clearOverride(key: string): void {
    this.overrides.delete(key);
  }

  warnings(): Warning[] {
    return [...this.warningsByKey.values()].flat();
  }

  warningsFor(key: string): Warning[] {
    return this.warningsByKey.get(key) ?? [];
  }

  box(): ViewBox {
    return this.view.box();
  }

  resetView(): void {
    this.view.reset();
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
    // A full fine request supersedes a pending render wait.
    if (quality === "fine" && this.waitRendered && keys.length === this.fns.length) this.waitRendered.id = id;
    this.post({ type: "sample", id, keys, C: this.constants(), range: this.computeRange(), quality });
    return id;
  }

  private onResult(r: FromWorker): void {
    if (r.quality === "coarse") this.coarseInFlight = false;
    for (const c of r.curves) {
      if (this.latest.get(c.key) !== r.id) continue;
      const f = this.state?.functions[c.key];
      if (!f) continue;
      this.curves.set(c.key, { result: c, range: r.range, quality: r.quality, version: f.version });
    }
    if (r.quality === "fine") {
      for (const key of new Set(r.curves.map((c) => c.key))) this.warningsByKey.delete(key);
      for (const w of r.warnings) {
        const k = w.source ?? "";
        this.warningsByKey.set(k, [...(this.warningsByKey.get(k) ?? []), { source: w.source, code: w.code, message: w.message }]);
      }
    }
    this.draw();
    if (this.waitRendered && r.id >= this.waitRendered.id && r.quality === "fine") {
      const rev = this.waitRendered.revision;
      this.waitRendered = null;
      this.cb.rendered(rev, this.warnings());
    }
  }

  /** After pan/zoom: resample if the view left the computed area or the zoom drifted. */
  private afterViewChange(interacting: boolean): void {
    if (!this.state || this.fns.length === 0) return;
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
    const batches: Batch[] = buildSpace(this.view);
    let seq = 0;
    const placed: Rect[] = [];
    for (const f of this.fns) {
      const c = this.curves.get(f.key);
      if (!c || this.hidden.has(f.key)) continue;
      const color = colorOf(f);
      const style = styles.length;
      styles.push({ color: rgba(color), width: 2.5, size: 7 });
      const base = { id: f.key, version: f.version, source: { def: f.key }, seq: seq++, style, origin: [c.result.origin[0], c.result.origin[1], 0] as [number, number, number], dim: 2 as const };
      if (c.result.kind === "points") {
        batches.push({ ...base, layer: 3, kind: "points", coords: c.result.coords, pointKind: new Uint8Array(c.result.coords.length / 2).fill(POINT_NORMAL) });
      } else {
        batches.push({ ...base, layer: 2, kind: "polyline", coords: c.result.coords, offsets: c.result.offsets });
      }
      const at = this.placeLabel(c.result, f.key.length, placed);
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
          labels: [{ kind: "tex", tex: identTex(f.name), px: 22 }],
          placement: [at.place],
        });
      }
    }
    const hit = this.hoverAt ? this.nearestPoint(this.hoverAt.px, this.hoverAt.py) : null;
    if (hit) {
      const style = styles.length;
      styles.push({ color: rgba(hit.color), width: 2.5, size: 11 });
      const label = styles.length;
      styles.push({ color: rgba("#ecedf0"), width: 0, size: 0 });
      batches.push({ id: "hover-point", version: 0, source: { def: hit.key }, layer: 3, seq: 1000, style, origin: [0, 0, 0], dim: 2, kind: "points", coords: Float32Array.from([hit.x, hit.y]), pointKind: Uint8Array.of(POINT_HOLE) });
      const d = this.decimals();
      const fmt = (v: number) => v.toFixed(d).replace("-", "\u2212");
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
        labels: [{ kind: "ui", text: `${hit.key}  (${fmt(hit.x)}, ${fmt(hit.y)})`, px: 13, weight: 600 }],
        placement: [{ align: right ? "left" : "right", baseline: "bottom", dx: right ? 12 : -12, dy: -10, halo: true }],
      });
    }
    this.renderer.draw(this.view, batches, styles);
  }

  /** Coordinate display precision for the current zoom. */
  private decimals(): number {
    return Math.max(0, Math.min(10, Math.ceil(Math.log10(this.view.scale)) + 1));
  }

  /**
   * Closest point of a visible curve to the pointer, within a few pixels. On polylines
   * the point is interpolated between samples at the projection of the pointer.
   */
  private nearestPoint(px: number, py: number): { key: string; color: string; x: number; y: number } | null {
    const v = this.view;
    const RADIUS = 12;
    let best: { key: string; color: string; x: number; y: number } | null = null;
    let bestD = RADIUS * RADIUS;
    for (const f of this.fns) {
      const c = this.curves.get(f.key);
      if (!c || this.hidden.has(f.key)) continue;
      const r = c.result;
      const [ox, oy] = r.origin;
      const X = (i: number) => ox + r.coords[2 * i]!, Y = (i: number) => oy + r.coords[2 * i + 1]!;
      const consider = (x: number, y: number) => {
        const dx = v.sx(x) - px, dy = v.sy(y) - py, d = dx * dx + dy * dy;
        if (d < bestD) {
          bestD = d;
          best = { key: f.key, color: colorOf(f), x, y };
        }
      };
      if (r.kind === "points") {
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
    const inside = (px: number, py: number) => px > 40 && px < v.width - 70 && py > 30 && py < v.height - 40;
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
    this.cursor.textContent = `x ${fmt(this.view.wx(px))} · y ${fmt(this.view.wy(py))}`;
  }
}
