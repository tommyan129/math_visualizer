// Workspace sidebar (UI doc "작업 공간 사이드바"): create, pick, ⋯ menu with key create/replace.
// The new key is shown once in an easy-to-select box (no clipboard API without HTTPS).

import type { WorkspaceInfo } from "../generated/WorkspaceInfo";
import { h, icon, ICONS } from "./dom";

export interface SidebarActions {
  open(id: string): void;
  create(name: string): Promise<string | null>;
  /** Returns the new key, or an error message. */
  replaceKey(id: string): Promise<{ key: string } | { error: string }>;
}

type MenuState = { id: string; step: "menu" | "confirm" | "busy" } | { id: string; step: "key"; key: string } | { id: string; step: "error"; message: string };

export class Sidebar {
  readonly el = h("nav", { class: "sidebar", "aria-label": "작업 공간" });
  private list: WorkspaceInfo[] = [];
  private active: string | null = null;
  private open = localStorage.getItem("mv.sidebar") !== "closed";
  private creating = false;
  private menu: MenuState | null = null;

  constructor(private actions: SidebarActions) {
    document.addEventListener("pointerdown", (e) => {
      if (this.menu && !(e.target as Element).closest(".ws-item")) {
        this.menu = null;
        this.render();
      }
    });
    this.render();
  }

  set(list: WorkspaceInfo[], active: string | null): void {
    this.list = list;
    this.active = active;
    this.render();
  }

  private toggle(): void {
    this.open = !this.open;
    localStorage.setItem("mv.sidebar", this.open ? "open" : "closed");
    this.menu = null;
    this.render();
  }

  private startCreate(): void {
    if (!this.open) this.toggle();
    this.creating = true;
    this.render();
  }

  private render(): void {
    const el = this.el;
    el.replaceChildren();
    el.classList.toggle("closed", !this.open);
    if (!this.open) {
      el.append(
        h("button", { class: "iconbtn", "aria-label": "사이드바 펼치기", onclick: () => this.toggle() }, icon(ICONS.expand)),
        h("button", { class: "iconbtn", "aria-label": "새 작업 공간", onclick: () => this.startCreate() }, icon(ICONS.plus)),
      );
      return;
    }
    el.append(
      h("div", { class: "sidebar-head" }, h("span", { class: "title" }, "작업 공간"), h("button", { class: "iconbtn", "aria-label": "사이드바 접기", onclick: () => this.toggle() }, icon(ICONS.collapse))),
    );
    if (this.creating) {
      const input = h("input", { class: "new-ws-input", placeholder: "이름 입력 후 Enter", maxlength: 80, "aria-label": "새 작업 공간 이름" });
      input.addEventListener("keydown", async (e) => {
        if (e.key === "Escape") {
          this.creating = false;
          this.render();
        } else if (e.key === "Enter" && input.value.trim()) {
          const err = await this.actions.create(input.value.trim());
          if (err) {
            input.setCustomValidity(err);
            input.reportValidity();
            return;
          }
          this.creating = false;
          this.render();
        }
      });
      input.addEventListener("blur", () => {
        if (!input.value.trim()) {
          this.creating = false;
          this.render();
        }
      });
      el.append(input);
      queueMicrotask(() => input.focus());
    } else {
      el.append(h("button", { class: "new-ws", onclick: () => this.startCreate() }, icon(ICONS.plus), "새 작업 공간"));
    }
    if (this.list.length > 0) el.append(h("div", { class: "recent" }, "최근"));
    const ul = h("div", { class: "ws-list" });
    for (const w of [...this.list].sort((a, b) => b.updated_at - a.updated_at)) ul.append(this.item(w));
    el.append(ul);
  }

  private item(w: WorkspaceInfo): HTMLElement {
    const menuOpen = this.menu?.id === w.id;
    const row = h(
      "div",
      { class: `ws${w.id === this.active ? " on" : ""}`, onclick: () => this.actions.open(w.id) },
      h("span", { class: "name", title: w.name }, w.name),
      w.has_key ? null : h("span", { class: "nokey" }, "key 없음"),
      h(
        "button",
        {
          class: `iconbtn small${menuOpen ? " open" : ""}`,
          "aria-label": "작업 공간 메뉴",
          onclick: (e: Event) => {
            e.stopPropagation();
            this.menu = menuOpen ? null : { id: w.id, step: "menu" };
            this.render();
          },
        },
        icon(ICONS.dots),
      ),
    );
    const wrap = h("div", { class: "ws-item" }, row);
    if (menuOpen && this.menu) wrap.append(this.menuEl(w, this.menu));
    return wrap;
  }

  private async replace(id: string): Promise<void> {
    this.menu = { id, step: "busy" };
    this.render();
    const r = await this.actions.replaceKey(id);
    this.menu = "key" in r ? { id, step: "key", key: r.key } : { id, step: "error", message: r.error };
    this.render();
  }

  private menuEl(w: WorkspaceInfo, m: MenuState): HTMLElement {
    const box = h("div", { class: "menu" });
    const label = w.has_key ? "key 교체" : "key 생성";
    box.append(
      h("button", { class: "menu-item", onclick: () => (w.has_key ? ((this.menu = { id: w.id, step: "confirm" }), this.render()) : void this.replace(w.id)) }, icon(ICONS.key), label),
    );
    if (m.step === "confirm") {
      box.append(h("div", { class: "menu-body" }, h("span", { class: "warn" }, "교체하면 이전 key는 즉시 무효가 됨"), h("button", { class: "pill-btn", onclick: () => void this.replace(w.id) }, "교체")));
    } else if (m.step === "busy") {
      box.append(h("div", { class: "menu-body" }, h("span", { class: "label" }, "만드는 중…")));
    } else if (m.step === "error") {
      box.append(h("div", { class: "menu-body" }, h("span", { class: "err" }, m.message)));
    } else if (m.step === "key") {
      const input = h("input", { class: "keybox", readonly: true, value: m.key, "aria-label": "새 key" });
      input.addEventListener("focus", () => input.select());
      box.append(h("div", { class: "menu-body" }, h("label", { class: "label" }, "새 key · 이전 key는 즉시 무효"), input, h("span", { class: "label" }, "이 key를 Claude에게 알려주세요. 다시 볼 수 없음.")));
      queueMicrotask(() => {
        input.focus();
        input.select();
      });
    }
    return box;
  }
}
