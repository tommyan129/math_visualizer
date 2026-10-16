// App wiring. The browser renders the resolved state model it receives; it never
// re-derives history or scoping rules. The server holds the current scene and step.

import "./style.css";
import type { Nav } from "./generated/Nav";
import type { Snapshot } from "./generated/Snapshot";
import type { UserOp } from "./generated/UserOp";
import type { Warning } from "./generated/Warning";
import type { WorkspaceInfo } from "./generated/WorkspaceInfo";
import { Plot } from "./canvas/plot";
import { Client, type ConnStatus } from "./net/client";
import { Animator } from "./ui/animate";
import { h, icon, ICONS } from "./ui/dom";
import { Drawer } from "./ui/drawer";
import { Panel } from "./ui/panel";
import { Sidebar } from "./ui/sidebar";
import { splitter } from "./ui/split";
import { StepBar } from "./ui/stepbar";
import { Topbar } from "./ui/topbar";

let conn: ConnStatus = "connecting";
let workspaces: WorkspaceInfo[] = [];
let openId: string | null = null;
try {
  openId = localStorage.getItem("mv.workspace");
} catch {
  openId = null;
}
let snapshot: Snapshot | null = null;

const wsUrl = `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`;

const client = new Client(wsUrl, {
  status(s) {
    conn = s;
    if (s === "connected" && openId) void openWorkspace(openId);
    render();
  },
  message(m) {
    if (m.type === "Workspaces") {
      workspaces = m.list;
      if (!openId || !workspaces.some((w) => w.id === openId)) {
        const recent = [...workspaces].sort((a, b) => b.updated_at - a.updated_at)[0];
        if (recent) void openWorkspace(recent.id);
        else openId = null;
      }
      render();
    } else if (m.type === "Snapshot") {
      onSnapshot(m.snapshot);
    } else if (m.type === "Navigate") {
      // Claude made a scene in another workspace: go there.
      void openWorkspace(m.workspace);
    }
  },
  serverRequest(r) {
    if (r.type === "GetView") {
      if (!openId) return { type: "Error", message: "열린 작업 공간 없음" };
      return { type: "View", workspace: openId, scene: nav().scene ?? "", view: plot.box(), warnings: plot.warnings(), spans: plot.spanDims() };
    }
    if (r.type === "Snapshot") {
      if (!openId) return { type: "Error", message: "열린 작업 공간 없음" };
      try {
        const png = plot.png(r.max_side);
        return { type: "Png", workspace: openId, ...png };
      } catch (e) {
        return { type: "Error", message: `캔버스를 PNG로 만들 수 없음 (${String(e)})` };
      }
    }
    return { type: "Error", message: "알 수 없는 요청" };
  },
});

function nav(): Nav {
  return snapshot?.nav ?? { scene: null, step: 0, updated: [] };
}

function currentScene() {
  const k = nav().scene;
  return (k && snapshot?.state.scenes[k]) || null;
}

async function openWorkspace(id: string): Promise<void> {
  if (openId !== id) {
    animator.stopAll();
    snapshot = null;
    openId = id;
    drawer.setRequests([]);
  }
  try {
    localStorage.setItem("mv.workspace", id);
  } catch {
    // Not remembered across reloads.
  }
  render();
  const r = await client.request({ type: "OpenWorkspace", workspace: id });
  if (r.type === "Error") console.warn(r.message);
}

function onSnapshot(s: Snapshot): void {
  if (s.workspace !== openId) return;
  // Late snapshots of an older revision are dropped.
  if (snapshot && snapshot.workspace === s.workspace && s.revision < snapshot.revision) return;
  const prev = snapshot?.nav;
  snapshot = s;
  // A step change we did not ask for (Claude edited the scene, another scene) stops playback.
  const ours = expectNav && s.nav.scene === expectNav.scene && s.nav.step === expectNav.step;
  if (ours) expectNav = null;
  else if (prev && (prev.scene !== s.nav.scene || prev.step !== s.nav.step)) animator.stopAll();
  for (const k of Object.keys(s.state.constants)) if (!panel.isDragging(k) && !animator.playing(k)) plot.clearOverride(k);
  plot.setState(s.state, s.revision, s.nav);
  render();
  void refreshRequests();
}

async function refreshRequests(): Promise<void> {
  if (!openId) return drawer.setRequests([]);
  const r = await client.request({ type: "GetRequests", workspace: openId });
  if (r.type === "Requests") drawer.setRequests(r.items);
}

async function applyDsl(source: string): Promise<{ lines: string[]; error: boolean }> {
  if (!openId) return { lines: ["열린 작업 공간 없음"], error: true };
  const r = await client.request({ type: "Apply", workspace: openId, source });
  if (r.type === "Applied") return { lines: r.lines, error: false };
  if (r.type === "Rejected") return { lines: [`적용 안 됨 (오류 ${r.lines.length}개)`, ...r.lines], error: true };
  return { lines: [r.type === "Error" ? r.message : "알 수 없는 응답"], error: true };
}

async function userOp(op: UserOp): Promise<string | null> {
  if (!openId) return "열린 작업 공간 없음";
  const r = await client.request({ type: "UserOp", workspace: openId, op });
  return r.type === "Error" ? r.message : null;
}

/** The scene and step we asked for; its snapshot must not stop the animation it started. */
let expectNav: { scene: string; step: number } | null = null;

/** Step navigation in the current scene. Forward into a step plays its animate. */
async function goStep(step: number): Promise<void> {
  const sc = currentScene();
  if (!openId || !sc) return;
  const from = nav().step;
  const to = Math.max(0, Math.min(step, sc.steps.length - 1));
  if (to === from) return;
  animator.stopAll();
  const anim = to > from ? (sc.steps[to]?.animate ?? []) : [];
  expectNav = { scene: sc.key, step: to };
  if (anim.length > 0) animator.start(anim);
  const r = await client.request({ type: "Navigate", workspace: openId, scene: sc.key, step: to });
  if (r.type === "Error") {
    expectNav = null;
    animator.stopAll();
    console.warn(r.message);
  }
}

async function goScene(key: string): Promise<void> {
  if (!openId || key === nav().scene) return;
  animator.stopAll();
  const r = await client.request({ type: "Navigate", workspace: openId, scene: key, step: null });
  if (r.type === "Error") console.warn(r.message);
}

// ----- layout -----

const sidebar = new Sidebar({
  open: (id) => void openWorkspace(id),
  async create(name) {
    const r = await client.request({ type: "CreateWorkspace", name });
    if (r.type === "Created") {
      await openWorkspace(r.workspace.id);
      return null;
    }
    return r.type === "Error" ? r.message : "만들 수 없음";
  },
  async replaceKey(id) {
    const r = await client.request({ type: "ReplaceKey", workspace: id });
    if (r.type === "Key") return { key: r.key };
    return { error: r.type === "Error" ? r.message : "실패" };
  },
});

const topbar = new Topbar(() => client.reclaim());

const animator = new Animator({
  frame(k, v) {
    plot.setConstant(k, v, false);
    panel.showValue(k, v);
  },
  done(k) {
    // The server holds the step's value; drop the local one and draw it finely.
    plot.releaseConstant(k);
    panel.endPreview(k);
    render();
  },
});

const panel = new Panel({
  toggle: (key, visible) => {
    const sc = nav().scene;
    if (sc) void userOp({ type: "Toggle", scene: sc, target: key, visible });
  },
  remove: (key) => void applyDsl(`del ${key}\n`),
  drag: (k, v) => {
    // Grabbing the slider stops its animation.
    animator.stop(k);
    plot.setConstant(k, v, false);
  },
  async setValue(k, v) {
    animator.stop(k);
    plot.setConstant(k, v, true);
    const e = await userOp({ type: "SetValue", constant: k, value: v });
    if (e) {
      plot.clearOverride(k);
      if (snapshot) plot.setState(snapshot.state, snapshot.revision, snapshot.nav);
    }
    return e;
  },
  setRange: (k, min, max) => userOp({ type: "SetRange", constant: k, min, max }),
  setClip: (src) => {
    const sc = nav().scene;
    return sc ? userOp({ type: "SetClip", scene: sc, src }) : Promise.resolve("scene 없음");
  },
});

const stepbar = new StepBar((step) => void goStep(step));

const empty = h("div", { class: "empty" });
const cursor = h("div", { class: "cursor", hidden: true });
const zoom = h(
  "div",
  { class: "zoom" },
  h("button", { class: "iconbtn", "aria-label": "확대", onclick: () => plot.zoom(1.5) }, icon(ICONS.plus)),
  h("button", { class: "iconbtn", "aria-label": "축소", onclick: () => plot.zoom(1 / 1.5) }, icon(ICONS.minus)),
  h("button", { class: "iconbtn", "aria-label": "처음 view로", onclick: () => plot.resetView() }, icon(ICONS.reset)),
);
const card = h("div", { class: "canvas-card" }, cursor, zoom, empty);
const drawer = new Drawer({
  apply: applyDsl,
  setDone: (number, done) => void userOp({ type: "SetRequestDone", number, done }),
});

const canvasCol = h("main", { class: "canvas-col" });
const splitPanel = splitter({
  dir: "x",
  key: "mv.split.panel",
  initial: 304,
  min: 220,
  max: () => Math.max(240, window.innerWidth * 0.5),
  sign: 1,
  apply: (w) => {
    panel.el.style.flexBasis = `${w}px`;
    panel.el.style.width = `${w}px`;
  },
});
const splitNote = splitter({
  dir: "y",
  key: "mv.split.note",
  initial: null,
  min: 44,
  max: () => Math.max(60, window.innerHeight * 0.45),
  sign: -1,
  apply: (px) => (stepbar.el.style.height = `${px}px`),
});
const splitDrawer = splitter({
  dir: "y",
  key: "mv.split.drawer",
  initial: 196,
  min: 64,
  max: () => Math.max(80, window.innerHeight * 0.6),
  sign: -1,
  apply: (px) => drawer.setHeight(px),
});
canvasCol.append(card, splitNote, stepbar.el, splitDrawer, drawer.el);

document.getElementById("app")!.replaceWith(
  h("div", { class: "app" }, sidebar.el, h("div", { class: "main" }, topbar.el, h("div", { class: "body" }, panel.el, splitPanel, canvasCol))),
);

const plot = new Plot(card, {
  rendered(revision: number, warnings: Warning[]) {
    // -1: the first-screen check finished after the render was reported.
    if (openId && revision >= 0) client.send({ type: "Rendered", workspace: openId, revision, warnings, spans: plot.spanDims() });
    render();
  },
});
// Keep overlays above the canvas.
card.append(cursor, zoom, empty);

document.addEventListener("keydown", (e) => {
  const t = e.target as HTMLElement | null;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
  if (e.altKey || e.ctrlKey || e.metaKey) return;
  if (!stepbar.visible) return;
  if (e.key === "ArrowRight" || e.key === "PageDown") {
    e.preventDefault();
    void goStep(nav().step + 1);
  } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
    e.preventDefault();
    void goStep(nav().step - 1);
  }
});

function render(): void {
  const ws = workspaces.find((w) => w.id === openId) ?? null;
  const state = snapshot?.state ?? null;
  const n = nav();
  const scene = currentScene();
  sidebar.set(workspaces, openId);
  topbar.render(ws?.name ?? null, conn, snapshot?.revision ?? null, {
    scenes: state ? Object.values(state.scenes).map((s) => s!) : [],
    current: n.scene,
    updated: new Set(n.updated),
    select: (k) => void goScene(k),
  });
  panel.render({ state, scene, step: n.step, hasKey: ws?.has_key ?? false, warnings: (k) => plot.warningsFor(k), sceneWarnings: plot.sceneWarnings(), spanDim: (k) => plot.spanDims().find((d) => d.key === k)?.dim });
  stepbar.render(state, scene, n.step);
  splitNote.hidden = !stepbar.visible;
  empty.textContent = !ws ? "작업 공간을 만들거나 고르면 그래프가 여기에 그려짐" : "";
  empty.hidden = !!ws;
}

render();
client.start();
