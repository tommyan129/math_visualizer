// Drawer under the canvas (UI doc "서랍"): DSL input and requests.
// The user's DSL goes to the server as source text, through the same path as Claude's.

import type { ReqView } from "../generated/ReqView";
import { h, icon, ICONS } from "./dom";

export interface DrawerActions {
  /** Resolves to result lines, or error lines with `error: true`. */
  apply(source: string): Promise<{ lines: string[]; error: boolean }>;
  setDone(number: number, done: boolean): void;
}

type Tab = "dsl" | "req";

function statusText(v: ReqView): { text: string; cls: string } {
  const s = v.status;
  switch (s.type) {
    case "Unchanged":
      return { text: "그대로", cls: "" };
    case "Modified":
      return { text: `수정됨 · 단계 ${s.seq}${s.via ? ` (${s.via} 변경으로)` : ""}`, cls: "changed" };
    case "Deleted":
      return { text: `삭제됨 · 단계 ${s.seq}${s.cause ? ` (${s.cause})` : ""}`, cls: "gone" };
  }
}

export class Drawer {
  readonly el = h("section", { class: "drawer", "aria-label": "DSL 입력, 요청사항" });
  private tab: Tab = "dsl";
  private openState = true;
  private body = h("div", { class: "drawer-body" });
  private tabDsl = h("button", { class: "dtab" }, "DSL 입력");
  private tabReq = h("button", { class: "dtab" });
  private input = h("textarea", { class: "dsl-input", "aria-label": "DSL 입력", spellcheck: "false", placeholder: 'req(f; 0.9 < x < 1.1; "여기 선이 끊겨 보임")' });
  private msg = h("div", { class: "dsl-msg" });
  private applyBtn = h("button", { class: "apply-btn" }, "적용");
  private reqList = h("div", { class: "reqs" });
  private items: ReqView[] = [];

  constructor(private actions: DrawerActions) {
    const chev = h("button", { class: "iconbtn", "aria-label": "서랍 열기/닫기", onclick: () => this.setOpen(!this.openState) }, icon(ICONS.chevron));
    this.tabDsl.addEventListener("click", () => this.show("dsl"));
    this.tabReq.addEventListener("click", () => this.show("req"));
    this.el.append(h("div", { class: "drawer-tabs" }, this.tabDsl, this.tabReq, h("div", { class: "spacer" }), chev), this.body);
    this.applyBtn.addEventListener("click", () => void this.submit());
    this.input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        void this.submit();
      }
      // Indentation is 4 spaces (tabs are an error in the DSL).
      if (e.key === "Tab") {
        e.preventDefault();
        this.input.setRangeText("    ", this.input.selectionStart, this.input.selectionEnd, "end");
      }
    });
    this.render();
  }

  /** Body height in px (splitter). */
  setHeight(px: number): void {
    this.body.style.height = `${px}px`;
  }

  private setOpen(v: boolean): void {
    this.openState = v;
    this.render();
  }

  private show(t: Tab): void {
    this.tab = t;
    this.openState = true;
    this.render();
  }

  private async submit(): Promise<void> {
    const src = this.input.value;
    if (!src.trim()) return;
    this.applyBtn.disabled = true;
    const r = await this.actions.apply(src.endsWith("\n") ? src : src + "\n");
    this.applyBtn.disabled = false;
    this.msg.textContent = r.lines.join("\n");
    this.msg.classList.toggle("err", r.error);
    if (!r.error) this.input.value = "";
  }

  setRequests(items: ReqView[]): void {
    this.items = [...items].sort((a, b) => b.number - a.number);
    this.render();
  }

  private render(): void {
    const pending = this.items.filter((v) => !v.done).length;
    this.tabReq.replaceChildren("요청사항", pending ? h("span", { class: "count" }, String(pending)) : "");
    this.tabDsl.classList.toggle("on", this.tab === "dsl");
    this.tabReq.classList.toggle("on", this.tab === "req");
    this.body.hidden = !this.openState;
    if (this.tab === "dsl") {
      if (!this.body.contains(this.input)) this.body.replaceChildren(h("div", { class: "dsl-box" }, this.input, h("div", { class: "dsl-foot" }, this.msg, this.applyBtn)));
      return;
    }
    this.reqList.replaceChildren(
      ...(this.items.length === 0
        ? [h("div", { class: "hint" }, "요청사항 없음. DSL 입력에서 req(f; 범위; \"메모\")로 남기면 Claude가 get_requests로 확인함.")]
        : this.items.map((v) => {
            const st = statusText(v);
            const box = h("input", { type: "checkbox", checked: v.done, "aria-label": `#${v.number} 처리됨`, title: "처리됨" });
            box.addEventListener("change", () => this.actions.setDone(v.number, box.checked));
            return h("div", { class: `req${v.done ? " done" : ""}` }, box, h("span", { class: "num" }, `#${v.number}`), h("code", {}, v.src), h("span", { class: `st ${st.cls}` }, st.text));
          })),
    );
    this.body.replaceChildren(this.reqList);
  }
}
