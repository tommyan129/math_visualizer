// 3D plot controller (stage 6a): free camera + worker + three.js renderer.
// - Math coordinates at 1:1:1. The camera has a position, a heading (yaw, pitch; z is
//   always up) and a focus distance (how far away the thing looked at is), which scales
//   moving, panning and the computed area (stage-6a decision, replacing the view box).
// - Bindings from the keymap (settings; defaults): left drag pans parallel to the screen,
//   right drag turns the view in place, W/A/S/D move (forward along the view), Space /
//   Shift move up / down; the wheel moves toward / away from the point under the
//   pointer. Keys act while the pointer is over the 3D canvas.
// - Computed area: the scene's `view` (−5..5 per missing axis), not drawn. Surfaces are
//   computed over its x, y ranges and cut at its z range (clipping planes); the camera
//   moves freely around it. (An area following the camera was tried: zooming out grew
//   x² − y² into walls filling the screen.)
// - The scene clip cuts per vertex (alpha test on an interpolated 0/1 attribute).
//   Highlights are cut on the surface where their region boundary crosses the mesh edges.
// - Labels (axis names, ticks, definition names) are drawn on a 2D canvas on top,
//   projected every frame, with the same MathJax label images as the 2D renderer.

import * as THREE from "three";
import { LineMaterial } from "three/addons/lines/LineMaterial.js";
import { LineSegments2 } from "three/addons/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/addons/lines/LineSegmentsGeometry.js";
import type { Expr } from "../generated/Expr";
import type { Function as Fn } from "../generated/Function";
import type { Nav } from "../generated/Nav";
import type { Scene } from "../generated/Scene";
import type { SpanDim } from "../generated/SpanDim";
import type { State } from "../generated/State";
import type { ViewBox } from "../generated/ViewBox";
import type { ViewSpec } from "../generated/ViewSpec";
import type { Warning } from "../generated/Warning";
import { identTex, isVectorName } from "../math/latex";
import { setImageReadyHandler, texImage } from "../math/mathjax";
import { compileConst, specsOf } from "../worker/codegen";
import type { Box3, MeshData, PatchData } from "../worker/mesh";
import { makeRuntime } from "../worker/numeric";
import type { FromWorker, Item3, ToWorker } from "../worker/worker";
import { actionsOf, bound, typing, type Action } from "../ui/keymap";
import { colorOf } from "./plot";
import { activeClip, visibleAt } from "./scene";
import { formatTick } from "./space";

const IDLE_MS = 180;
/** Axis range when the view does not give one (stage-6a decision). */
const DEFAULT_RANGE: [number, number] = [-5, 5];
const FOV = 45;
const BG = "#212329";
const GRID = "#31333b";
const AXIS = "#858993";
const TICK = "#a3a6af";
const LABEL = "#ecedf0";
const HALO = "#212329";
/** Opacity of a span plane / the whole-space box. */
const PLANE_ALPHA = 0.18;
/** Least opacity of a surface highlight (fill opacities are tuned for flat fills). */
const HL_MIN_ALPHA = 0.55;
const LINE_PX = 2.2;
/** Axes reach this many computed-area half sizes past the area. */
const AXIS_REACH = 6;
/** Label occlusion is checked this long after the camera stops. */
const OCCLUDE_MS = 120;
const GRID_STORE = "mv.grid3d";
/** On entry the view's corners reach this far toward the canvas edges (NDC, 1 = edge). */
const FIT = 0.85;
/** Moving speed: focus distances per second (× the speed toggle). */
const MOVE_SPEED = 0.8;
/** Speed toggle steps. */
const SPEEDS = [1, 2, 5];
/** Turning: radians per pixel of mouse movement. */
const TURN = 0.004;

const CONST_R = makeRuntime();

export interface Plot3DCallbacks {
  /** All shown definitions of `revision` are drawn at fine quality. */
  rendered(revision: number, warnings: Warning[]): void;
}

interface Cam {
  pos: THREE.Vector3;
  /** Heading of the view: yaw around z, pitch up from the xy plane. */
  yaw: number;
  pitch: number;
  /** Distance to the looked-at point. */
  focus: number;
  /** Computed area (the view). */
  area: Box3;
}

/** Entry heading: from the front right, a little above. */
const ENTRY = { yaw: (120 * Math.PI) / 180, pitch: (-24 * Math.PI) / 180 };
const MAX_PITCH = 1.55;

/** The box for a `view` spec: one range per axis, -5..5 where it gives none. */
export function boxOf(spec: ViewSpec | null): Box3 {
  const ev = (r: { lo: Expr; hi: Expr } | null): [number, number] => {
    if (!r) return DEFAULT_RANGE;
    try {
      const a = compileConst(r.lo, CONST_R)(), b = compileConst(r.hi, CONST_R)();
      return Number.isFinite(a) && Number.isFinite(b) && b > a ? [a, b] : DEFAULT_RANGE;
    } catch {
      return DEFAULT_RANGE;
    }
  };
  const [x0, x1] = ev(spec?.h ?? null), [y0, y1] = ev(spec?.v ?? null), [z0, z1] = ev(spec?.d ?? null);
  return { x0, x1, y0, y1, z0, z1 };
}

/** Nice tick step (1·2·5 × 10ⁿ) for about `count` intervals over `span`. */
function tickStep(span: number, count = 5): { step: number; decimals: number } {
  const raw = span / count;
  const p = Math.pow(10, Math.floor(Math.log10(raw)));
  const m = [1, 2, 5, 10].find((k) => k * p >= raw * 0.999) ?? 10;
  const step = m * p;
  return { step, decimals: Math.max(0, -Math.floor(Math.log10(step) + 1e-9)) };
}

/** Label drawn on the overlay at a projected point. */
interface Label3 {
  /** Point in math coordinates. */
  at: THREE.Vector3;
  tex?: string;
  text?: string;
  color: string;
  px: number;
  /** Screen offset in px. */
  dx: number;
  dy: number;
}

/** Drawn objects of one definition (or highlight). */
interface Obj {
  group: THREE.Group;
  item: Item3;
  labels: Label3[];
  version: string;
  quality: "coarse" | "fine";
}

export class Plot3D {
  private host: HTMLElement;
  private canvas: HTMLCanvasElement;
  private overlay: HTMLCanvasElement;
  private cursor: HTMLElement;
  private renderer: THREE.WebGLRenderer;
  private scene3 = new THREE.Scene();
  private camera = new THREE.PerspectiveCamera(FOV, 1, 0.1, 100);
  private world = new THREE.Group();
  /** Floor grid, axes, ticks (rebuilt with the computed area). */
  private frame = new THREE.Group();
  /** Arrow heads and span points (sized by the focus distance). */
  private marks = new THREE.Group();
  /** z range of the computed area (set with it). */
  private planes = [new THREE.Plane(new THREE.Vector3(0, 0, -1), 0), new THREE.Plane(new THREE.Vector3(0, 0, 1), 0)];
  private worker: Worker;
  private width = 0;
  private height = 0;
  private active = false;

  private state: State | null = null;
  private sceneObj: Scene | null = null;
  private sceneKey: string | null = null;
  private step = 0;
  private axes: string[] = ["x", "y", "z"];
  private axisTex: string[] = ["x", "y", "z"];
  private fns: Fn[] = [];
  private hidden = new Set<string>();
  private constKeys: string[] = [];
  private overrides = new Map<string, number>();

  private cam: Cam = { pos: new THREE.Vector3(), ...ENTRY, focus: 1, area: boxOf(null) };
  private cams = new Map<string, Cam>();
  private appliedView = new Map<string, string>();
  /** Camera waiting for the canvas to have a size (fit needs the aspect). */
  private pendingFit: Box3 | null = null;
  /** Keys held (movement), and whether the pointer is over the canvas. */
  private keys = new Set<string>();
  private hovering = false;
  private moving = 0;
  /** Index into SPEEDS. */
  private speed = 0;
  private lastFrame = 0;
  /** Frame labels behind a surface at the resting camera. */
  private occluded = new Set<Label3>();
  private occludeTimer = 0;
  /** Grid toggles (canvas buttons), remembered per workspace and scene in this browser. */
  private grids = { floor: true, third: false };
  private gridKey: string | null = null;
  private gridBox: HTMLElement;

  private objs = new Map<string, Obj>();
  private frameLabels: Label3[] = [];
  private warningsByKey = new Map<string, Warning[]>();
  private entryWarnings: Warning[] = [];
  private entryCheck: string | null = null;
  private reqId = 0;
  private latest = new Map<string, number>();
  private waitRendered: { id: number; revision: number } | null = null;
  private coarseInFlight = false;
  private pending = new Set<string>();
  private idleTimer = 0;
  private drawQueued = false;
  private hlStamp = "";
  private clipStamp = "";

  constructor(host: HTMLElement, private cb: Plot3DCallbacks) {
    this.host = host;
    this.canvas = document.createElement("canvas");
    this.canvas.className = "plot-canvas plot-gl";
    this.overlay = document.createElement("canvas");
    this.overlay.className = "plot-canvas plot-3d";
    this.canvas.hidden = true;
    this.overlay.hidden = true;
    host.append(this.canvas, this.overlay);
    this.cursor = host.querySelector(".cursor") as HTMLElement;
    this.renderer = new THREE.WebGLRenderer({ canvas: this.canvas, antialias: true });
    this.renderer.setClearColor(BG);
    this.renderer.localClippingEnabled = true;
    this.camera.up.set(0, 0, 1);
    this.camera.rotation.order = "ZYX";
    this.scene3.add(this.world, this.frame, this.marks, this.camera);
    this.scene3.add(new THREE.AmbientLight(0xffffff, 1.6));
    const sun = new THREE.DirectionalLight(0xffffff, 1.9);
    sun.position.set(-1, 2, 3);
    this.camera.add(sun);
    this.worker = new Worker(new URL("../worker/worker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (e: MessageEvent<FromWorker>) => this.onResult(e.data);
    setImageReadyHandler(() => this.queueDraw());
    new ResizeObserver(() => {
      const r = host.getBoundingClientRect();
      this.resize(r.width, r.height);
    }).observe(host);
    this.gridBox = document.createElement("div");
    this.gridBox.className = "grid-toggles";
    this.gridBox.hidden = true;
    host.append(this.gridBox);
    this.bindInput();
  }

  /** Scene changed: load its grid toggles. */
  private setGridKey(key: string | null): void {
    if (key === this.gridKey) {
      this.renderGridButtons();
      return;
    }
    this.gridKey = key;
    let saved: Partial<typeof this.grids> = {};
    try {
      saved = (JSON.parse(localStorage.getItem(GRID_STORE) ?? "{}") as Record<string, typeof this.grids>)[key ?? ""] ?? {};
    } catch {
      saved = {};
    }
    this.grids = { floor: saved.floor ?? true, third: saved.third ?? false };
    this.buildFrame();
    this.renderGridButtons();
  }

  private toggleGrid(which: "floor" | "third"): void {
    this.grids[which] = !this.grids[which];
    try {
      const all = JSON.parse(localStorage.getItem(GRID_STORE) ?? "{}") as Record<string, typeof this.grids>;
      if (this.gridKey) all[this.gridKey] = { ...this.grids };
      localStorage.setItem(GRID_STORE, JSON.stringify(all));
    } catch {
      // Kept for this page only.
    }
    this.buildFrame();
    this.renderGridButtons();
    this.draw();
  }

  /** Two buttons under the zoom buttons: floor grid, third-axis grid (named by the axes). */
  private renderGridButtons(): void {
    const [a, b, c] = this.axes.map((x) => x.replace(/^\\/, ""));
    const btn = (which: "floor" | "third", label: string, title: string) => {
      const el = document.createElement("button");
      el.className = `grid-btn${this.grids[which] ? " on" : ""}`;
      el.textContent = label;
      el.title = title;
      el.setAttribute("aria-pressed", String(this.grids[which]));
      el.addEventListener("click", () => this.toggleGrid(which));
      return el;
    };
    const speed = document.createElement("button");
    speed.className = `grid-btn speed${this.speed ? " on" : ""}`;
    speed.textContent = `×${SPEEDS[this.speed]}`;
    speed.title = "이동 속도 (누르거나 Shift: ×1 → ×2 → ×5)";
    speed.addEventListener("click", () => this.cycleSpeed());
    this.gridBox.replaceChildren(btn("floor", `${a}${b}`, `바닥 격자 (${a}${b} 평면) 켜기·끄기`), btn("third", `${c}`, `${c}축 격자 (세로 평면) 켜기·끄기`), speed);
  }

  /** Moving speed ×1 → ×2 → ×5 → ×1. */
  private cycleSpeed(): void {
    this.speed = (this.speed + 1) % SPEEDS.length;
    this.renderGridButtons();
  }

  /** Shown (3D scene) or hidden (2D scene). */
  setActive(on: boolean): void {
    if (on === this.active) return;
    this.active = on;
    this.canvas.hidden = !on;
    this.overlay.hidden = !on;
    this.gridBox.hidden = !on;
    if (on) this.queueDraw();
  }

  private resize(w: number, h: number): void {
    if (w <= 0 || h <= 0) return;
    this.width = w;
    this.height = h;
    const dpr = window.devicePixelRatio || 1;
    this.renderer.setPixelRatio(dpr);
    this.renderer.setSize(w, h, false);
    this.overlay.width = Math.round(w * dpr);
    this.overlay.height = Math.round(h * dpr);
    this.camera.aspect = w / h;
    for (const g of [this.world, this.frame]) g.traverse((c) => c instanceof LineSegments2 && (c.material as LineMaterial).resolution.set(w, h));
    if (this.pendingFit) this.fitView(this.pendingFit);
    this.placeCamera();
    this.draw();
  }

  // ----- model -----------------------------------------------------------

  setState(state: State, revision: number, nav: Nav, workspace = ""): void {
    const prevState = this.state;
    this.state = state;
    const scene = (nav.scene && state.scenes[nav.scene]) || null;
    this.enterScene(scene);
    this.sceneObj = scene;
    this.step = scene ? Math.min(nav.step, scene.steps.length - 1) : 0;
    this.axes = scene?.axes.map((a) => a.key) ?? ["x", "y", "z"];
    this.axisTex = scene?.axes.map((a) => identTex(a)) ?? ["x", "y", "z"];
    this.setGridKey(scene ? `${workspace}|${scene.key}` : null);
    const members = new Set(scene?.members ?? []);
    const all = Object.values(state.functions).map((f) => f!).sort((a, b) => a.order - b.order);
    this.fns = all.filter((f) => members.has(f.key) && f.kind !== "Calc");
    this.hidden = new Set([...Object.keys(state.functions), ...Object.keys(state.highlights)].filter((k) => !scene || !visibleAt(scene, this.step, k)));
    this.constKeys = Object.keys(state.constants).sort();
    for (const k of [...this.objs.keys()]) if (!this.fns.some((f) => f.key === k)) this.removeObj(k);
    for (const k of [...this.warningsByKey.keys()]) if (!state.functions[k]) this.warningsByKey.delete(k);
    const moved = this.applyViewRule();

    this.post({ type: "functions", fns: specsOf(state, all), constKeys: this.constKeys });
    const clip = activeClip(scene);
    const clipStamp = JSON.stringify([this.axes, clip?.expr ?? null]);
    const clipChanged = clipStamp !== this.clipStamp;
    if (clipChanged) {
      this.clipStamp = clipStamp;
      this.post({ type: "scene", axes: this.axes, polar: false, clip: clip?.expr ?? null });
    }
    // Surface highlights: computed with their surface.
    const hls = Object.values(state.highlights).map((h) => h!).filter((h) => h.target.type === "Function" && members.has(h.key));
    this.post({ type: "highlights", specs: hls.map((h) => ({ key: h.key, fn: h.target.type === "Function" ? h.target.function : null, range: h.range })) });
    const hlStamp = JSON.stringify(hls.map((h) => [h.key, h.version]));
    const hlTargets = new Set(hls.filter((h) => !prevState || prevState.highlights[h.key]?.version !== h.version).map((h) => (h.target.type === "Function" ? h.target.function : "")));
    if (hlStamp !== this.hlStamp) for (const k of Object.keys(prevState?.highlights ?? {})) if (!state.highlights[k]) hlTargets.add(this.targetOf(prevState!, k));
    this.hlStamp = hlStamp;

    const changedConsts = new Set(this.constKeys.filter((k) => !prevState || prevState.constants[k]?.version !== state.constants[k]!.version));
    const clipConsts = new Set(clip?.consts ?? []);
    const dirty = this.fns
      .filter((f) => {
        const o = this.objs.get(f.key);
        return moved || clipChanged || !o || o.version !== this.depStamp(f.key) || hlTargets.has(f.key) || f.consts.some((k) => changedConsts.has(k)) || [...clipConsts].some((k) => changedConsts.has(k)) || this.hlConstsChanged(f.key, changedConsts);
      })
      .map((f) => f.key);
    this.applyVisibility();
    if (dirty.length === 0) {
      this.draw();
      this.cb.rendered(revision, this.warnings());
      this.checkEntry();
      return;
    }
    const id = this.request(dirty, "fine");
    this.waitRendered = { id, revision };
  }

  private targetOf(state: State, hl: string): string {
    const t = state.highlights[hl]?.target;
    return t && t.type === "Function" ? t.function : "";
  }

  private hlConstsChanged(fn: string, changed: Set<string>): boolean {
    return Object.values(this.state?.highlights ?? {}).some((h) => h!.target.type === "Function" && h!.target.function === fn && h!.consts.some((k) => changed.has(k)));
  }

  /** Versions of a definition and of everything it refers to (as in the 2D plot). */
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

  /** Scene switch: keep the old camera, restore the new scene's. */
  private enterScene(scene: Scene | null): void {
    const key = scene?.key ?? null;
    if (key === this.sceneKey) return;
    if (this.sceneKey) this.cams.set(this.sceneKey, cloneCam(this.cam));
    this.sceneKey = key;
    this.entryWarnings = [];
    this.entryCheck = null;
    for (const k of [...this.objs.keys()]) this.removeObj(k);
    if (!key) return;
    const cam = this.cams.get(key);
    if (cam) this.setCam(cam);
    else this.entryCheck = key;
  }

  /** The view of the current step differs from the one last applied: move the camera. */
  private applyViewRule(): boolean {
    if (!this.sceneObj) return false;
    const spec = this.sceneObj.steps[this.step]?.view ?? null;
    const src = spec?.src ?? "";
    if (this.appliedView.get(this.sceneObj.key) === src) return false;
    this.appliedView.set(this.sceneObj.key, src);
    this.fitView(boxOf(spec));
    return true;
  }

  private setCam(c: Cam): void {
    this.cam = cloneCam(c);
    this.setArea(this.cam.area);
    this.placeCamera();
  }

  /**
   * Camera for a view box: the entry heading, far enough that the box's corners fit the
   * canvas; the box is the computed area.
   */
  private fitView(b: Box3): void {
    if (this.width === 0) {
      this.pendingFit = b;
      this.cam.area = { ...b };
      this.setArea(b);
      return;
    }
    this.pendingFit = null;
    const c = new THREE.Vector3((b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2, (b.z0 + b.z1) / 2);
    const f = forward(ENTRY.yaw, ENTRY.pitch);
    const corners: THREE.Vector3[] = [];
    for (const x of [b.x0, b.x1]) for (const y of [b.y0, b.y1]) for (const z of [b.z0, b.z1]) corners.push(new THREE.Vector3(x, y, z));
    const r = corners[0]!.distanceTo(c);
    let lo = r * 0.2, hi = r * 40;
    for (let i = 0; i < 40; i++) {
      const mid = (lo + hi) / 2;
      this.cam = { ...this.cam, pos: c.clone().addScaledVector(f, -mid), ...ENTRY, focus: mid };
      this.placeCamera();
      const ext = Math.max(...corners.map((p) => {
        const v = p.clone().project(this.camera);
        return v.z > 1 || v.z < -1 ? Infinity : Math.max(Math.abs(v.x), Math.abs(v.y));
      }));
      if (ext > FIT) lo = mid;
      else hi = mid;
    }
    this.cam = { ...this.cam, pos: c.clone().addScaledVector(f, -hi), ...ENTRY, focus: hi };
    this.setArea(b);
    this.placeCamera();
  }

  /** The computed area: frame, z cut. */
  private setArea(b: Box3): void {
    this.cam.area = { ...b };
    this.planes[0]!.constant = b.z1 + (b.z1 - b.z0) * 1e-4;
    this.planes[1]!.constant = -b.z0 + (b.z1 - b.z0) * 1e-4;
    this.buildFrame();
    this.layoutMarks();
  }

  /** First screen: definitions with nothing inside the box. */
  private checkEntry(): void {
    if (!this.entryCheck || this.entryCheck !== this.sceneKey) return;
    const shown = this.fns.filter((f) => !this.hidden.has(f.key));
    if (shown.some((f) => this.objs.get(f.key)?.quality !== "fine")) return;
    this.entryCheck = null;
    const out: Warning[] = [];
    for (const f of shown) {
      const o = this.objs.get(f.key)!;
      if (!this.anyInBox(o.item)) out.push({ source: f.key, code: "W-offscreen", message: "처음 화면(view 범위)에 보이는 점 없음" });
    }
    this.entryWarnings = out;
    this.cb.rendered(-1, this.warnings());
  }

  private anyInBox(it: Item3): boolean {
    const b = this.cam.area;
    const inBox = (x: number, y: number, z: number) => x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1 && z >= b.z0 && z <= b.z1;
    const scan = (p: Float32Array, o: number[], alpha?: Float32Array) => {
      for (let i = 0; i < p.length; i += 3) if ((!alpha || alpha[i / 3]) && inBox(p[i]! + o[0]!, p[i + 1]! + o[1]!, p[i + 2]! + o[2]!)) return true;
      return false;
    };
    switch (it.kind) {
      case "mesh":
        return it.meshes.some((m) => m.indices.length > 0 && scan(m.positions, m.origin, m.alpha));
      case "lines":
        return scan(it.line.positions, it.line.origin);
      case "span":
        return true;
      case "arrow":
        return scan(it.positions, it.origin);
    }
  }

  // ----- constants ---------------------------------------------------------

  setConstant(key: string, value: number, final: boolean): void {
    this.overrides.set(key, value);
    if (!this.state || !this.active) return;
    const keys = this.fns.filter((f) => f.consts.includes(key) || this.hlConstsChanged(f.key, new Set([key])) || (activeClip(this.sceneObj)?.consts ?? []).includes(key)).map((f) => f.key);
    if (keys.length === 0) return;
    if (final) {
      this.request(keys, "fine");
      return;
    }
    if (this.coarseInFlight) keys.forEach((k) => this.pending.add(k));
    else this.request(keys, "coarse");
    this.scheduleFine(keys);
  }

  clearOverride(key: string): void {
    this.overrides.delete(key);
  }

  releaseConstant(key: string): void {
    this.overrides.delete(key);
    if (!this.active) return;
    const keys = this.fns.filter((f) => f.consts.includes(key)).map((f) => f.key);
    if (keys.length) this.request(keys, "fine");
  }

  private constants(): Float64Array {
    const C = new Float64Array(this.constKeys.length);
    this.constKeys.forEach((k, i) => (C[i] = this.overrides.get(k) ?? this.state!.constants[k]!.value));
    return C;
  }

  // ----- worker ------------------------------------------------------------

  private post(m: ToWorker): void {
    this.worker.postMessage(m);
  }

  private request(keys: string[], quality: "coarse" | "fine"): number {
    const id = ++this.reqId;
    for (const k of keys) this.latest.set(k, id);
    if (quality === "coarse") this.coarseInFlight = true;
    this.post({ type: "sample3d", id, keys, C: this.constants(), box: { ...this.cam.area }, quality });
    return id;
  }

  private scheduleFine(keys: string[]): void {
    const all = new Set(keys);
    window.clearTimeout(this.idleTimer);
    this.idleTimer = window.setTimeout(() => this.request([...all, ...this.pending], "fine"), IDLE_MS);
  }

  private onResult(r: FromWorker): void {
    if (r.type !== "result3d") return;
    if (r.quality === "coarse") this.coarseInFlight = false;
    const done = new Set<string>();
    for (const it of r.items) {
      if (this.latest.get(it.key) !== r.id) continue;
      this.setObj(it, r.quality);
      done.add(it.key);
    }
    // Keys that produced nothing (an error, an empty range) drop their old geometry.
    for (const [k, id] of this.latest) if (id === r.id && !done.has(k)) this.removeObj(k);
    for (const k of new Set(r.warnings.map((w) => w.source).filter((s): s is string => !!s))) this.warningsByKey.delete(k);
    for (const [k, id] of this.latest) if (id === r.id) this.warningsByKey.delete(k);
    for (const w of r.warnings) {
      const k = w.source ?? "";
      this.warningsByKey.set(k, [...(this.warningsByKey.get(k) ?? []), { source: w.source, code: w.code, message: w.message }]);
    }
    this.applyVisibility();
    this.draw();
    if (r.quality === "coarse" && this.pending.size) {
      const keys = [...this.pending];
      this.pending.clear();
      this.request(keys, "coarse");
    }
    if (this.waitRendered && r.id >= this.waitRendered.id && r.quality === "fine") {
      const rev = this.waitRendered.revision;
      this.waitRendered = null;
      this.cb.rendered(rev, this.warnings());
    }
    if (r.quality === "fine") this.checkEntry();
  }

  // ----- objects -------------------------------------------------------------

  private removeObj(key: string): void {
    const o = this.objs.get(key);
    if (!o) return;
    o.group.parent?.remove(o.group);
    o.group.traverse((c) => {
      if (c instanceof THREE.Mesh || c instanceof THREE.LineSegments || c instanceof LineSegments2) {
        c.geometry.dispose();
        (c.material as THREE.Material).dispose();
      }
    });
    this.objs.delete(key);
  }

  private colorOf(key: string): string {
    const f = this.state?.functions[key];
    return f ? colorOf(f) : LABEL;
  }

  private setObj(it: Item3, quality: "coarse" | "fine"): void {
    this.removeObj(it.key);
    const color = this.colorOf(it.key);
    const group = new THREE.Group();
    const labels: Label3[] = [];
    const f = this.state!.functions[it.key]!;
    const nameTex = identTex(f.name, undefined, isVectorName(f));
    switch (it.kind) {
      case "mesh": {
        for (const m of it.meshes) group.add(this.surfaceMesh(m, color));
        for (const h of it.hls) {
          const hl = this.state!.highlights[h.key];
          if (!hl || it.meshes.length !== 1) continue;
          const patch = this.patchMesh(h.patch, hl.style.color, Math.max(HL_MIN_ALPHA, hl.style.opacity));
          patch.userData.hl = h.key;
          group.add(patch);
        }
        const m = it.meshes[0];
        const at = m && this.labelVertex(m);
        if (at) labels.push({ at, tex: nameTex, color, px: 22, dx: 8, dy: -8 });
        break;
      }
      case "lines": {
        const segs = segmentsOf(it.line.positions, it.line.offsets);
        if (segs.length) {
          const line = this.fatLines(segs, color, LINE_PX);
          line.position.set(...it.line.origin);
          group.add(line);
          // Name at the last point inside the computed area.
          const p = it.line.positions, og = it.line.origin, b = this.cam.area;
          for (let i = p.length / 3 - 1; i >= 0; i--) {
            const x = p[i * 3]! + og[0], y = p[i * 3 + 1]! + og[1], z = p[i * 3 + 2]! + og[2];
            if (x < b.x0 || x > b.x1 || y < b.y0 || y > b.y1 || z < b.z0 || z > b.z1) continue;
            labels.push({ at: new THREE.Vector3(x, y, z), tex: nameTex, color, px: 22, dx: 8, dy: -8 });
            break;
          }
        }
        break;
      }
      case "span": {
        const p = it.positions;
        if (it.dim === 1 && p.length === 6) {
          const line = this.fatLines(Array.from(p), color, LINE_PX);
          line.position.set(...it.origin);
          group.add(line);
          labels.push({ at: new THREE.Vector3(p[3]! + it.origin[0], p[4]! + it.origin[1], p[5]! + it.origin[2]), tex: nameTex, color, px: 22, dx: 8, dy: -8 });
        } else if (it.dim === 2 && p.length >= 9) {
          const geo = new THREE.BufferGeometry();
          geo.setAttribute("position", new THREE.BufferAttribute(p, 3));
          const idx: number[] = [];
          for (let i = 1; i + 1 < p.length / 3; i++) idx.push(0, i, i + 1);
          geo.setIndex(idx);
          const mat = new THREE.MeshBasicMaterial({ color, transparent: true, opacity: PLANE_ALPHA, side: THREE.DoubleSide, depthWrite: false, clippingPlanes: this.planes });
          const mesh = new THREE.Mesh(geo, mat);
          mesh.position.set(...it.origin);
          group.add(mesh);
          const edge = this.fatLines(ringSegments(p), color, 1.2);
          edge.position.set(...it.origin);
          group.add(edge);
          labels.push({ at: new THREE.Vector3(p[0]! + it.origin[0], p[1]! + it.origin[1], p[2]! + it.origin[2]), tex: nameTex, color, px: 22, dx: 8, dy: -8 });
        }
        break;
      }
      case "arrow":
        // The head is a mark sized by the focus distance.
        const shaft = this.fatLines(Array.from(it.positions), color, LINE_PX);
        shaft.position.set(...it.origin);
        group.add(shaft);
        // Beyond the head of the (first) arrow.
        if (it.positions.length >= 6) labels.push({ at: new THREE.Vector3(it.positions[3]! + it.origin[0], it.positions[4]! + it.origin[1], it.positions[5]! + it.origin[2]), tex: nameTex, color, px: 22, dx: 10, dy: -10 });
        break;
    }
    this.world.add(group);
    this.objs.set(it.key, { group, item: it, labels, version: this.depStamp(it.key), quality });
    this.layoutMarks();
  }

  /** A vertex near the far corner of the grid that is drawn, for the name label. */
  private labelVertex(m: MeshData): THREE.Vector3 | null {
    const stride = m.n + 1;
    for (let k = 0; k < 12; k++) {
      const i = Math.round(m.n * (0.85 - 0.05 * k)), j = Math.round(m.m * (0.85 - 0.05 * k));
      const v = j * stride + i;
      if (m.alpha[v]) {
        const p = new THREE.Vector3(m.positions[v * 3]! + m.origin[0], m.positions[v * 3 + 1]! + m.origin[1], m.positions[v * 3 + 2]! + m.origin[2]);
        const b = this.cam.area;
        if (p.z >= b.z0 && p.z <= b.z1) return p;
      }
    }
    return null;
  }

  private surfaceMesh(m: MeshData, color: string): THREE.Group {
    const g = new THREE.Group();
    g.position.set(...m.origin);
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(m.positions, 3));
    geo.setAttribute("normal", new THREE.BufferAttribute(m.normals, 3));
    // Brightness changes slightly with height (stage-6a decision); alpha is the clip.
    const b = this.cam.area;
    const rgba = new Float32Array(m.alpha.length * 4);
    for (let v = 0; v < m.alpha.length; v++) {
      const z = m.positions[v * 3 + 2]! + m.origin[2];
      const t = Math.min(1, Math.max(0, (z - b.z0) / (b.z1 - b.z0 || 1)));
      const k = 0.82 + 0.18 * t;
      rgba[v * 4] = k;
      rgba[v * 4 + 1] = k;
      rgba[v * 4 + 2] = k;
      rgba[v * 4 + 3] = m.alpha[v]!;
    }
    geo.setAttribute("color", new THREE.BufferAttribute(rgba, 4));
    geo.setIndex(new THREE.BufferAttribute(m.indices, 1));
    const mat = new THREE.MeshLambertMaterial({ color, vertexColors: true, side: THREE.DoubleSide, alphaTest: 0.5, clippingPlanes: this.planes, polygonOffset: true, polygonOffsetFactor: 1, polygonOffsetUnits: 1 });
    g.add(new THREE.Mesh(geo, mat));
    if (m.wire.length) {
      const wg = new THREE.BufferGeometry();
      wg.setAttribute("position", new THREE.BufferAttribute(m.wire, 3));
      g.add(new THREE.LineSegments(wg, new THREE.LineBasicMaterial({ color: "#000000", transparent: true, opacity: 0.16, clippingPlanes: this.planes })));
    }
    return g;
  }

  /** The part of a surface a highlight covers, drawn over it in the highlight color. */
  private patchMesh(p: PatchData, color: string, opacity: number): THREE.Mesh {
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(p.positions, 3));
    geo.setAttribute("normal", new THREE.BufferAttribute(p.normals, 3));
    // Alpha is the clip, cut per vertex like the surface under it.
    const rgba = new Float32Array(p.alpha.length * 4);
    for (let v = 0; v < p.alpha.length; v++) {
      rgba[v * 4] = 1;
      rgba[v * 4 + 1] = 1;
      rgba[v * 4 + 2] = 1;
      rgba[v * 4 + 3] = p.alpha[v]!;
    }
    geo.setAttribute("color", new THREE.BufferAttribute(rgba, 4));
    const mat = new THREE.MeshLambertMaterial({ color, vertexColors: true, transparent: true, opacity, alphaTest: opacity * 0.5, side: THREE.DoubleSide, depthWrite: false, clippingPlanes: this.planes, polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -4 });
    const mesh = new THREE.Mesh(geo, mat);
    mesh.position.set(...p.origin);
    return mesh;
  }

  private fatLines(segs: ArrayLike<number>, color: string, px: number, clip = true): LineSegments2 {
    const geo = new LineSegmentsGeometry();
    geo.setPositions(Array.from(segs));
    const mat = new LineMaterial({ color, linewidth: px, worldUnits: false, clippingPlanes: clip ? this.planes : [] });
    mat.resolution.set(this.width || 1, this.height || 1);
    return new LineSegments2(geo, mat);
  }

  /** Visibility from the scene and step (toggles hide without recomputing). */
  private applyVisibility(): void {
    for (const [k, o] of this.objs) {
      o.group.visible = !this.hidden.has(k);
      o.group.traverse((c) => {
        if (c.userData.hl) c.visible = !this.hidden.has(c.userData.hl as string);
      });
    }
    this.layoutMarks();
  }

  // ----- frame, camera -----------------------------------------------------------

  /**
   * The three axes through the origin, reaching well past the computed area; ticks along
   * them inside the area's ranges; a faint grid on z = 0 over the area.
   */
  private buildFrame(): void {
    for (const c of [...this.frame.children]) {
      this.frame.remove(c);
      if (c instanceof THREE.LineSegments || c instanceof LineSegments2) {
        c.geometry.dispose();
        (c.material as THREE.Material).dispose();
      }
    }
    const b = this.cam.area;
    const labels: Label3[] = [];
    const tx = tickStep(b.x1 - b.x0), ty = tickStep(b.y1 - b.y0), tz = tickStep(b.z1 - b.z0);
    const ticksOf = (lo: number, hi: number, step: number) => {
      const out: number[] = [];
      for (let v = Math.ceil(lo / step) * step; v <= hi + 1e-9 * step; v += step) out.push(v);
      return out;
    };
    const xs = ticksOf(b.x0, b.x1, tx.step), ys = ticksOf(b.y0, b.y1, ty.step), zs = ticksOf(b.z0, b.z1, tz.step);
    const grid: number[] = [];
    // Floor: the plane of the first two axes at 0 (when 0 is inside the third range).
    if (this.grids.floor && b.z0 <= 0 && 0 <= b.z1) {
      for (const x of xs) grid.push(x, b.y0, 0, x, b.y1, 0);
      for (const y of ys) grid.push(b.x0, y, 0, b.x1, y, 0);
    }
    // Third axis: the two upright planes through the origin (first axis = 0, second = 0),
    // a line at each height tick and at each tick of the other axis.
    if (this.grids.third) {
      if (b.y0 <= 0 && 0 <= b.y1) {
        for (const z of zs) grid.push(b.x0, 0, z, b.x1, 0, z);
        for (const x of xs) grid.push(x, 0, b.z0, x, 0, b.z1);
      }
      if (b.x0 <= 0 && 0 <= b.x1) {
        for (const z of zs) grid.push(0, b.y0, z, 0, b.y1, z);
        for (const y of ys) grid.push(0, y, b.z0, 0, y, b.z1);
      }
    }
    if (grid.length) {
      const gg = new THREE.BufferGeometry();
      gg.setAttribute("position", new THREE.Float32BufferAttribute(grid, 3));
      this.frame.add(new THREE.LineSegments(gg, new THREE.LineBasicMaterial({ color: GRID })));
    }
    const ranges: [number, number][] = [[b.x0, b.x1], [b.y0, b.y1], [b.z0, b.z1]];
    const axes: number[] = [];
    ranges.forEach(([lo, hi], a) => {
      const reach = (hi - lo) * AXIS_REACH;
      const p = [0, 0, 0], q = [0, 0, 0];
      p[a] = Math.min(lo, 0) - reach;
      q[a] = Math.max(hi, 0) + reach;
      axes.push(...p, ...q);
      // Name at the area's positive end (or just past the origin when the area is below it).
      const n = [0, 0, 0];
      n[a] = hi > 0 ? hi : (hi - lo) * 0.1;
      labels.push({ at: new THREE.Vector3(...n), tex: this.axisTex[a], color: LABEL, px: 20, dx: 10, dy: -10 });
    });
    this.frame.add(this.fatLines(axes, AXIS, 1.6, false));
    const ticks: [number, { step: number; decimals: number }][] = [[0, tx], [1, ty], [2, tz]];
    const marks: number[] = [];
    for (const [a, t] of ticks) {
      const [lo, hi] = ranges[a]!;
      const len = t.step * 0.12;
      for (let v = Math.ceil(lo / t.step) * t.step; v <= hi + 1e-9 * t.step; v += t.step) {
        if (Math.abs(v) < t.step * 1e-6) continue;
        const m = [0, 0, 0];
        m[a] = v;
        const off = a === 2 ? [len, 0, 0] : [0, 0, len];
        marks.push(m[0]! - off[0]!, m[1]! - off[1]!, m[2]! - off[2]!, m[0]! + off[0]!, m[1]! + off[1]!, m[2]! + off[2]!);
        labels.push({ at: new THREE.Vector3(...m), text: formatTick(v, t.decimals), color: TICK, px: 11, dx: a === 2 ? -16 : 0, dy: a === 2 ? 0 : 12 });
      }
    }
    if (marks.length) this.frame.add(this.fatLines(marks, AXIS, 1.4, false));
    this.frameLabels = labels;
    this.occluded = new Set();
  }

  /** Arrow heads and span points, sized by the focus distance. */
  private layoutMarks(): void {
    for (const c of [...this.marks.children]) {
      this.marks.remove(c);
      if (c instanceof THREE.Mesh) {
        c.geometry.dispose();
        (c.material as THREE.Material).dispose();
      }
    }
    const f = this.cam.focus;
    for (const [k, o] of this.objs) {
      if (this.hidden.has(k)) continue;
      const it = o.item;
      const color = this.colorOf(k);
      if (it.kind === "arrow") {
        // One head per arrow (a `family` has many).
        const p = it.positions, og = it.origin;
        for (let i = 0; i + 5 < p.length; i += 6) {
          const tail = new THREE.Vector3(p[i]! + og[0], p[i + 1]! + og[1], p[i + 2]! + og[2]), head = new THREE.Vector3(p[i + 3]! + og[0], p[i + 4]! + og[1], p[i + 5]! + og[2]);
          const dir = head.clone().sub(tail);
          const len = dir.length();
          if (len < 1e-12) continue;
          const h = Math.min(f * 0.045, len * 0.4);
          const cone = new THREE.Mesh(new THREE.ConeGeometry(h * 0.38, h, 18), new THREE.MeshLambertMaterial({ color, clippingPlanes: this.planes }));
          cone.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir.clone().normalize());
          cone.position.copy(head.clone().sub(dir.clone().normalize().multiplyScalar(h / 2)));
          this.marks.add(cone);
        }
      } else if (it.kind === "span" && it.dim === 0) {
        const s = new THREE.Mesh(new THREE.SphereGeometry(f * 0.012, 16, 12), new THREE.MeshLambertMaterial({ color }));
        s.position.set(it.origin[0] + it.positions[0]!, it.origin[1] + it.positions[1]!, it.origin[2] + it.positions[2]!);
        this.marks.add(s);
      } else if (it.kind === "span" && it.dim === 3) {
        // All of space: the computed area, faintly.
        const a = this.cam.area;
        const m = new THREE.Mesh(new THREE.BoxGeometry(a.x1 - a.x0, a.y1 - a.y0, a.z1 - a.z0), new THREE.MeshBasicMaterial({ color, transparent: true, opacity: PLANE_ALPHA * 0.6, depthWrite: false, side: THREE.DoubleSide }));
        m.position.set((a.x0 + a.x1) / 2, (a.y0 + a.y1) / 2, (a.z0 + a.z1) / 2);
        this.marks.add(m);
      }
    }
  }

  /** three.js camera from the state: position, heading, near / far by the focus distance. */
  private placeCamera(): void {
    const { pos, yaw, pitch, focus } = this.cam;
    this.camera.position.copy(pos);
    this.camera.lookAt(pos.clone().add(forward(yaw, pitch)));
    this.camera.near = focus / 200;
    this.camera.far = focus * 200;
    this.camera.updateProjectionMatrix();
    this.camera.updateMatrixWorld(true);
  }

  // ----- interaction -----------------------------------------------------------

  /** The computed area (`get_state`). */
  box(): ViewBox {
    const b = this.cam.area;
    return { x_min: b.x0, x_max: b.x1, y_min: b.y0, y_max: b.y1, z_min: b.z0, z_max: b.z1 };
  }

  /** Back to the current step's view. */
  resetView(): void {
    this.fitView(boxOf(this.sceneObj?.steps[this.step]?.view ?? null));
    this.afterAreaChange();
    this.draw();
  }

  /** Zoom buttons: toward / away from the middle of the canvas (factor > 1 = closer). */
  zoom(factor: number): void {
    this.dolly(this.width / 2, this.height / 2, 1 / factor);
    this.draw();
  }

  /** Resample for a new area (reset to the view). */
  private afterAreaChange(): void {
    const keys = this.fns.map((f) => f.key);
    if (keys.length === 0) return;
    window.clearTimeout(this.idleTimer);
    this.request(keys, "fine");
  }

  /** Surfaces under the pointer (math coordinates), nearest first. */
  private pick(px: number, py: number): THREE.Intersection | null {
    const ray = new THREE.Raycaster();
    ray.setFromCamera(new THREE.Vector2((px / this.width) * 2 - 1, -(py / this.height) * 2 + 1), this.camera);
    const meshes: THREE.Object3D[] = [];
    for (const [k, o] of this.objs) if (!this.hidden.has(k) && o.item.kind === "mesh") o.group.traverse((c) => c instanceof THREE.Mesh && !c.userData.hl && meshes.push(c));
    const a = this.cam.area;
    return ray.intersectObjects(meshes, false).find((h) => h.point.z >= a.z0 && h.point.z <= a.z1) ?? null;
  }

  /**
   * Wheel: move toward (k < 1) or away from (k > 1) the point under the pointer — the
   * surface hit there, else the point at the focus distance along that ray. The heading
   * stays, so that point stays under the pointer.
   */
  private dolly(px: number, py: number, k: number): void {
    const hit = this.pick(px, py);
    let target: THREE.Vector3;
    if (hit) target = hit.point.clone();
    else {
      const ray = new THREE.Raycaster();
      ray.setFromCamera(new THREE.Vector2((px / this.width) * 2 - 1, -(py / this.height) * 2 + 1), this.camera);
      target = ray.ray.origin.clone().addScaledVector(ray.ray.direction, this.cam.focus);
    }
    this.cam.pos = target.clone().addScaledVector(this.cam.pos.clone().sub(target), k);
    this.cam.focus *= k;
    this.placeCamera();
    // Arrow heads and points keep their size on screen.
    this.layoutMarks();
  }

  /** Drag: the scene follows the pointer (the plane at the focus distance moves with it). */
  private pan(dx: number, dy: number): void {
    const perPx = (2 * this.cam.focus * Math.tan((FOV * Math.PI) / 360)) / (this.height || 1);
    const right = new THREE.Vector3().setFromMatrixColumn(this.camera.matrixWorld, 0);
    const up = new THREE.Vector3().setFromMatrixColumn(this.camera.matrixWorld, 1);
    this.cam.pos.addScaledVector(right, -dx * perPx).addScaledVector(up, dy * perPx);
    this.placeCamera();
  }

  /** Turn the view in place (the turn binding). */
  private turn(dx: number, dy: number): void {
    this.cam.yaw -= dx * TURN;
    this.cam.pitch = Math.max(-MAX_PITCH, Math.min(MAX_PITCH, this.cam.pitch - dy * TURN));
    this.placeCamera();
  }

  /** Movement keys held: move every frame until released. */
  private step3(t: number): void {
    const dt = Math.min(0.1, (t - this.lastFrame) / 1000);
    this.lastFrame = t;
    const f = forward(this.cam.yaw, this.cam.pitch);
    const right = new THREE.Vector3(f.y, -f.x, 0).normalize();
    const v = new THREE.Vector3();
    const held = (a: Action) => [...this.keys].some((k) => bound(a, k));
    if (held("forward")) v.add(f);
    if (held("back")) v.sub(f);
    if (held("right")) v.add(right);
    if (held("left")) v.sub(right);
    if (held("up")) v.z += 1;
    if (held("down")) v.z -= 1;
    if (v.lengthSq() === 0) {
      this.moving = 0;
      return;
    }
    this.cam.pos.addScaledVector(v.normalize(), this.cam.focus * MOVE_SPEED * SPEEDS[this.speed]! * dt);
    this.placeCamera();
    this.draw();
    this.moving = requestAnimationFrame((tt) => this.step3(tt));
  }

  /** Cursor: arrow, crosshair over a surface, grabbing while moving, four arrows while turning. */
  private setMode(mode: "idle" | "over" | "pan" | "turn"): void {
    const el = this.overlay;
    for (const m of ["over", "pan", "turn"]) el.classList.toggle(m, m === mode);
  }

  /**
   * Input by the keymap (settings): a pan / turn binding that is a mouse button acts
   * while dragging with it; one that is a key acts while the key is held and the mouse
   * moves. Movement keys act while held. Keys count only while the pointer is over the
   * canvas and nothing is being typed.
   */
  private bindInput(): void {
    const el = this.overlay;
    let drag: { x: number; y: number; act: "pan" | "turn"; bit: number } | null = null;
    const end = () => {
      if (!drag) return;
      drag = null;
      this.setMode("idle");
    };
    el.addEventListener("contextmenu", (e) => e.preventDefault());
    // Keep the browser's own handling of a bound button (menus, gestures) out where it can be.
    el.addEventListener("mousedown", (e) => {
      if (bound("turn", `Mouse${e.button}`) || bound("pan", `Mouse${e.button}`)) e.preventDefault();
    });
    el.addEventListener("pointerenter", () => (this.hovering = true));
    el.addEventListener("pointerdown", (e) => {
      const code = `Mouse${e.button}`;
      const act = bound("turn", code) ? "turn" : bound("pan", code) ? "pan" : null;
      if (!act) return;
      e.preventDefault();
      // `buttons` bit of this button (left 1, right 2, middle 4).
      const bit = e.button === 1 ? 4 : e.button === 2 ? 2 : 1 << e.button;
      drag = { x: e.clientX, y: e.clientY, act, bit };
      el.setPointerCapture(e.pointerId);
      this.cursor.hidden = true;
      this.setMode(act);
    });
    el.addEventListener("pointermove", (e) => {
      this.hovering = true;
      // The release can be lost (a browser gesture or menu took the button up): end the
      // drag as soon as a move shows the button is no longer down.
      if (drag && !(e.buttons & drag.bit)) end();
      const keyAct = [...this.keys].some((k) => bound("turn", k)) ? "turn" : [...this.keys].some((k) => bound("pan", k)) ? "pan" : null;
      const act = drag?.act ?? keyAct;
      if (!act) {
        const r = el.getBoundingClientRect();
        this.hover(e.clientX - r.left, e.clientY - r.top);
        return;
      }
      const dx = drag ? e.clientX - drag.x : e.movementX, dy = drag ? e.clientY - drag.y : e.movementY;
      if (drag) {
        drag.x = e.clientX;
        drag.y = e.clientY;
      }
      this.cursor.hidden = true;
      this.setMode(act);
      if (act === "turn") this.turn(dx, dy);
      else this.pan(dx, dy);
      this.queueDraw();
    });
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
    el.addEventListener("lostpointercapture", end);
    window.addEventListener("blur", end);
    el.addEventListener("pointerleave", () => {
      this.cursor.hidden = true;
      this.hovering = false;
      this.keys.clear();
      if (!drag) this.setMode("idle");
    });
    el.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        const r = el.getBoundingClientRect();
        this.dolly(e.clientX - r.left, e.clientY - r.top, Math.exp(e.deltaY * 0.0015));
        this.queueDraw();
      },
      { passive: false },
    );
    const MOVES: Action[] = ["forward", "back", "left", "right", "up", "down"];
    window.addEventListener("keydown", (e) => {
      if (!this.active || !this.hovering || e.ctrlKey || e.metaKey || e.altKey || typing(e)) return;
      const acts = actionsOf(e.code).filter((a) => a !== "stepNext" && a !== "stepPrev");
      if (acts.length === 0) return;
      e.preventDefault();
      if (acts.includes("speed") && !e.repeat) this.cycleSpeed();
      this.keys.add(e.code);
      if (acts.some((a) => MOVES.includes(a)) && !this.moving) {
        this.lastFrame = performance.now();
        this.moving = requestAnimationFrame((tt) => this.step3(tt));
      }
    });
    window.addEventListener("keyup", (e) => {
      this.keys.delete(e.code);
      if (!drag && ![...this.keys].some((k) => bound("turn", k) || bound("pan", k))) this.setMode("idle");
    });
    window.addEventListener("blur", () => this.keys.clear());
  }

  /** Hover: the surface point under the pointer. */
  private hover(px: number, py: number): void {
    const hit = this.pick(px, py);
    this.setMode(hit ? "over" : "idle");
    if (!hit) {
      this.cursor.hidden = true;
      return;
    }
    let key = "";
    for (const [k, o] of this.objs) {
      let found = false;
      o.group.traverse((c) => (found ||= c === hit.object));
      if (found) key = k;
    }
    const p = hit.point;
    const a = this.cam.area;
    const d = tickStep(Math.max(a.x1 - a.x0, a.y1 - a.y0, a.z1 - a.z0), 500).decimals;
    const fmt = (v: number) => (Math.abs(v) < 0.5 * 10 ** -d ? 0 : v).toFixed(d).replace("-", "−");
    const [ax, ay, az] = this.axes.map((a) => a.replace(/^\\/, ""));
    this.cursor.hidden = false;
    this.cursor.textContent = `${key} · ${ax} ${fmt(p.x)} · ${ay} ${fmt(p.y)} · ${az} ${fmt(p.z)}`;
  }

  // ----- drawing ---------------------------------------------------------------

  private queueDraw(): void {
    if (this.drawQueued || !this.active) return;
    this.drawQueued = true;
    requestAnimationFrame(() => {
      this.drawQueued = false;
      this.draw();
    });
  }

  private draw(): void {
    if (!this.active || this.width === 0) return;
    this.renderer.render(this.scene3, this.camera);
    this.drawLabels();
    window.clearTimeout(this.occludeTimer);
    this.occludeTimer = window.setTimeout(() => this.checkOcclusion(), OCCLUDE_MS);
  }

  /**
   * Axis names and ticks behind a surface are hidden (labels are drawn over the WebGL
   * image). Checked once the camera rests: a ray per label against the drawn surfaces.
   */
  private checkOcclusion(): void {
    if (!this.active) return;
    const meshes: THREE.Object3D[] = [];
    for (const [k, o] of this.objs) if (!this.hidden.has(k) && o.item.kind === "mesh") o.group.traverse((c) => c instanceof THREE.Mesh && !c.userData.hl && meshes.push(c));
    const next = new Set<Label3>();
    if (meshes.length) {
      const ray = new THREE.Raycaster();
      const eye = this.camera.position;
      for (const l of this.frameLabels) {
        const d = l.at.clone().sub(eye);
        const dist = d.length();
        ray.set(eye, d.normalize());
        ray.far = dist - 1e-3;
        const a = this.cam.area;
        const hit = ray.intersectObjects(meshes, false).find((h) => h.point.z >= a.z0 && h.point.z <= a.z1);
        if (hit) next.add(l);
      }
    }
    const same = next.size === this.occluded.size && [...next].every((l) => this.occluded.has(l));
    this.occluded = next;
    if (!same) this.drawLabels();
  }

  private drawLabels(): void {
    const ctx = this.overlay.getContext("2d")!;
    const dpr = window.devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, this.width, this.height);
    const all = this.frameLabels.filter((l) => !this.occluded.has(l));
    for (const [k, o] of this.objs) {
      if (this.hidden.has(k)) continue;
      all.push(...o.labels);
    }
    for (const l of all) {
      const v = l.at.clone().project(this.camera);
      if (v.z > 1 || v.z < -1) continue;
      const x = ((v.x + 1) / 2) * this.width + l.dx, y = ((1 - v.y) / 2) * this.height + l.dy;
      if (l.text !== undefined) {
        ctx.font = `${l.px}px "JetBrains Mono", monospace`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.lineWidth = 3;
        ctx.strokeStyle = HALO;
        ctx.strokeText(l.text, x, y);
        ctx.fillStyle = l.color;
        ctx.fillText(l.text, x, y);
        continue;
      }
      const img = texImage(l.tex!, l.color, l.px, HALO, dpr);
      if (!img.ready) continue;
      const left = Math.round((x - img.width / 2) * dpr) / dpr, top = Math.round((y - img.height / 2) * dpr) / dpr;
      ctx.drawImage(img.halo, left, top, img.width, img.height);
      ctx.drawImage(img.img, left, top, img.width, img.height);
    }
  }

  // ----- reports ---------------------------------------------------------------

  warnings(): Warning[] {
    return [...this.entryWarnings, ...[...this.warningsByKey.values()].flat()];
  }

  warningsFor(key: string): Warning[] {
    return this.warnings().filter((w) => w.source === key);
  }

  sceneWarnings(): Warning[] {
    return this.warnings().filter((w) => w.source === null);
  }

  /** Dimension of each shown span as last drawn at fine quality. */
  spanDims(): SpanDim[] {
    return this.fns
      .filter((f) => f.kind === "Span" && !this.hidden.has(f.key))
      .flatMap((f) => {
        const o = this.objs.get(f.key);
        return o && o.item.kind === "span" ? [{ key: f.key, dim: o.item.dim }] : [];
      });
  }

  /** The WebGL image and the labels as one PNG. */
  png(maxSide: number): { data: string; width: number; height: number } {
    this.draw();
    const k = Math.min(1, maxSide / Math.max(this.canvas.width, this.canvas.height));
    const w = Math.max(1, Math.round(this.canvas.width * k)), h = Math.max(1, Math.round(this.canvas.height * k));
    const out = document.createElement("canvas");
    out.width = w;
    out.height = h;
    const ctx = out.getContext("2d")!;
    ctx.drawImage(this.canvas, 0, 0, w, h);
    ctx.drawImage(this.overlay, 0, 0, w, h);
    const url = out.toDataURL("image/png");
    return { data: url.slice(url.indexOf(",") + 1), width: w, height: h };
  }
}

/** Unit view direction for a heading (z up). */
function forward(yaw: number, pitch: number): THREE.Vector3 {
  return new THREE.Vector3(Math.cos(pitch) * Math.cos(yaw), Math.cos(pitch) * Math.sin(yaw), Math.sin(pitch));
}

function cloneCam(c: Cam): Cam {
  return { ...c, pos: c.pos.clone(), area: { ...c.area } };
}

/** Polyline pieces → segment pairs. */
function segmentsOf(p: Float32Array, offsets: Uint32Array): number[] {
  const out: number[] = [];
  for (let k = 0; k + 1 < offsets.length; k++) {
    for (let i = offsets[k]!; i + 1 < offsets[k + 1]!; i++) out.push(p[i * 3]!, p[i * 3 + 1]!, p[i * 3 + 2]!, p[i * 3 + 3]!, p[i * 3 + 4]!, p[i * 3 + 5]!);
  }
  return out;
}

/** Closed ring → segment pairs. */
function ringSegments(p: Float32Array): number[] {
  const n = p.length / 3;
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    const j = (i + 1) % n;
    out.push(p[i * 3]!, p[i * 3 + 1]!, p[i * 3 + 2]!, p[j * 3]!, p[j * 3 + 1]!, p[j * 3 + 2]!);
  }
  return out;
}
