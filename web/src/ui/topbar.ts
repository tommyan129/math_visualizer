// Top bar: workspace name, scene tabs (stage 1: only the default `2D(x, y)`), connection.

import type { ConnStatus } from "../net/client";
import { texElement } from "../math/mathjax";
import { h } from "./dom";

export class Topbar {
  readonly el = h("header", { class: "topbar" });

  constructor(private reclaim: () => void) {}

  render(name: string | null, conn: ConnStatus, revision: number | null): void {
    const tab = h("button", { class: "stab on", role: "tab", "aria-selected": "true" }, texElement("\\mathrm{2D}(x, y)"));
    const tabs = h("div", { class: "tabs", role: "tablist", "aria-label": "scene" }, tab);
    const parts: Node[] = [h("span", { class: "ws-name" }, name ?? "작업 공간 없음")];
    if (name) parts.push(tabs);
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
