// App wiring. The browser renders the resolved state model it receives; it never
// re-derives history or scoping rules.

import "./style.css";
import type { Snapshot } from "./generated/Snapshot";
import type { UserOp } from "./generated/UserOp";
import type { Warning } from "./generated/Warning";
import type { WorkspaceInfo } from "./generated/WorkspaceInfo";
import { Plot } from "./canvas/plot";
import { Client, type ConnStatus } from "./net/client";
import { h, icon, ICONS } from "./ui/dom";
import { Drawer } from "./ui/drawer";
import { Panel } from "./ui/panel";
import { Sidebar } from "./ui/sidebar";
import { Topbar } from "./ui/topbar";

const DEFAULT_SCENE = "2D(x, y)";

let conn: ConnStatus = "connecting";
let workspaces: WorkspaceInfo[] = [];
let openId: string | null = localStorage.getItem("mv.workspace");
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
    }
  },
  serverRequest(r) {
    if (r.type === "GetView") {
      if (!openId) return { type: "Error", message: "열린 작업 공간 없음" };
      return { type: "View", workspace: openId, scene: DEFAULT_SCENE, view: plot.box(), warnings: plot.warnings() };
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

async function openWorkspace(id: string): Promise<void> {
  if (openId !== id) {
    snapshot = null;
    openId = id;
    drawer.setRequests([]);
  }
  localStorage.setItem("mv.workspace", id);
  render();
  const r = await client.request({ type: "OpenWorkspace", workspace: id });
  if (r.type === "Error") console.warn(r.message);
}

function onSnapshot(s: Snapshot): void {
  if (s.workspace !== openId) return;
  // Late snapshots of an older revision are dropped.
  if (snapshot && snapshot.workspace === s.workspace && s.revision < snapshot.revision) return;
  snapshot = s;
  for (const k of Object.keys(s.state.constants)) if (!panel.isDragging(k)) plot.clearOverride(k);
  plot.setState(s.state, s.revision);
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

const panel = new Panel({
  toggle: (key, visible) => void userOp({ type: "Toggle", scene: DEFAULT_SCENE, target: key, visible }),
  remove: (key) => void applyDsl(`del ${key}\n`),
  drag: (k, v) => plot.setConstant(k, v, false),
  async setValue(k, v) {
    plot.setConstant(k, v, true);
    const e = await userOp({ type: "SetValue", constant: k, value: v });
    if (e) {
      plot.clearOverride(k);
      if (snapshot) plot.setState(snapshot.state, snapshot.revision);
    }
    return e;
  },
  setRange: (k, min, max) => userOp({ type: "SetRange", constant: k, min, max }),
});

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

document.getElementById("app")!.replaceWith(
  h(
    "div",
    { class: "app" },
    sidebar.el,
    h("div", { class: "main" }, topbar.el, h("div", { class: "body" }, panel.el, h("main", { class: "canvas-col" }, card, drawer.el))),
  ),
);

const plot = new Plot(card, {
  rendered(revision: number, warnings: Warning[]) {
    if (openId) client.send({ type: "Rendered", workspace: openId, revision, warnings });
    render();
  },
});
// Keep overlays above the canvas.
card.append(cursor, zoom, empty);

function render(): void {
  const ws = workspaces.find((w) => w.id === openId) ?? null;
  sidebar.set(workspaces, openId);
  topbar.render(ws?.name ?? null, conn, snapshot?.revision ?? null);
  panel.render(snapshot?.state ?? null, ws?.has_key ?? false, (k) => plot.warningsFor(k));
  empty.textContent = !ws ? "작업 공간을 만들거나 고르면 그래프가 여기에 그려짐" : "";
  empty.hidden = !!ws;
}

render();
client.start();
