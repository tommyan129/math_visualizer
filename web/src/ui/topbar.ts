// Top bar: workspace name, scene tabs (default scenes by coordinate system, Claude
// scenes by number, a yellow dot when a scene changed while not shown), connection.

import type { Scene } from "../generated/Scene";
import type { ConnStatus } from "../net/client";
import { sceneAxes, sortedScenes } from "../canvas/scene";
import { texElement } from "../math/mathjax";
import { h } from "./dom";

export interface TopbarScenes {
  scenes: Scene[];
  current: string | null;
  updated: Set<string>;
  select(key: string): void;
}

export class Topbar {
  readonly el = h("header", { class: "topbar" });

  constructor(private reclaim: () => void) {}

  render(name: string | null, conn: ConnStatus, revision: number | null, sc: TopbarScenes): void {
    const tabs = h("div", { class: "tabs", role: "tablist", "aria-label": "scene" });
    for (const s of sortedScenes(sc.scenes)) {
      const on = s.key === sc.current;
      const [, tex] = sceneAxes(s);
      const label: Node = s.number === null ? texElement(`\\mathrm{${s.polar ? "polar" : "2D"}}(${tex[0]}, ${tex[1]})`) : document.createTextNode(`scene ${s.number}`);
      const dot = sc.updated.has(s.key) ? h("span", { class: "updated", "aria-label": "갱신됨", title: "갱신됨" }) : null;
      tabs.append(h("button", { class: `stab${on ? " on" : ""}`, role: "tab", "aria-selected": on ? "true" : "false", onclick: () => sc.select(s.key) }, label, dot));
    }
    const parts: Node[] = [h("span", { class: "ws-name" }, name ?? "작업 공간 없음")];
    if (name && sc.scenes.length > 0) parts.push(tabs);
    parts.push(h("div", { class: "spacer" }), this.conn(conn, revision));
    this.el.replaceChildren(...parts);
  }

  private conn(c: ConnStatus, revision: number | null): HTMLElement {
    const [dot, text] = (
      {
        connected: ["ok", "연결됨"],
        connecting: ["warn", "연결 중…"],
        disconnected: ["err", "끊김 · 다시 연결 중"],
        "taken-over": ["warn", "다른 탭에서 열림"],
        reload: ["err", "새로고침 필요"],
      } as const
    )[c];
    const action =
      c === "taken-over"
        ? h("button", { onclick: () => this.reclaim() }, "여기서 열기")
        : c === "reload"
          ? h("button", { onclick: () => location.reload() }, "새로고침")
          : null;
    return h(
      "div",
      { class: "conn", role: "status" },
      h("span", { class: `dot ${dot}` }),
      text,
      action,
      revision !== null && c === "connected" ? h("span", { class: "rev" }, `rev ${revision}`) : null,
    );
  }
}
