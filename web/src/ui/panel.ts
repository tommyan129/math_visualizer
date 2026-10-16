// Definition and constant panel (UI doc "정의·토글 패널", "상수 슬라이더").
// Definitions: checkbox + expression (function name in its graph color), warnings below.
// Constants: one card each; drag updates locally, release sends the final value.

import type { Constant } from "../generated/Constant";
import type { Function as Fn } from "../generated/Function";
import type { State } from "../generated/State";
import type { Warning } from "../generated/Warning";
import { colorOf } from "../canvas/plot";
import { definitionTex, identTex, type LatexCtx } from "../math/latex";
import { texElement } from "../math/mathjax";
import { h, icon, ICONS } from "./dom";

export interface PanelActions {
  toggle(fn: string, visible: boolean): void;
  drag(constant: string, value: number): void;
  /** Resolves to an error message, or null. */
  setValue(constant: string, value: number): Promise<string | null>;
  setRange(constant: string, min: number, max: number): Promise<string | null>;
}

function decimals(step: number): number {
  return Math.max(0, Math.ceil(-Math.log10(step) - 1e-9));
}

function fmt(v: number, step: number): string {
  return v.toFixed(decimals(step)).replace("-", "−");
}

/** Range ends: no forced decimals (−10, 2.5). */
function fmtEnd(v: number): string {
  return String(Number(v.toFixed(6))).replace("-", "\u2212");
}

function parse(s: string): number {
  return Number(s.trim().replace("−", "-"));
}

class ConstantCard {
  readonly el: HTMLElement;
  private val: HTMLInputElement;
  private rng: HTMLInputElement;
  private fill: HTMLElement;
  private min: HTMLInputElement;
  private max: HTMLInputElement;
  private err: HTMLElement;
  private c: Constant;
  dragging = false;

  constructor(c: Constant, private actions: PanelActions) {
    this.c = c;
    const id = `c-${c.order}`;
    this.val = h("input", { class: "val", "aria-label": `${c.key} 값`, inputmode: "decimal" });
    this.rng = h("input", { class: "rng", type: "range", id });
    this.fill = h("div", { class: "fill" });
    this.min = h("input", { class: "end", "aria-label": `${c.key} 최솟값`, inputmode: "decimal" });
    this.max = h("input", { class: "end max", "aria-label": `${c.key} 최댓값`, inputmode: "decimal" });
    this.err = h("div", { class: "err", hidden: true });
    const name = h("label", { class: "cname", for: id }, texElement(identTex(c.name)));
    this.el = h(
      "div",
      { class: "const" },
      h("div", { class: "top" }, name, this.val),
      h("div", { class: "track" }, h("div", { class: "rail" }, this.fill), this.rng),
      h("div", { class: "ends" }, this.min, this.max),
      this.err,
    );

    this.rng.addEventListener("input", () => {
      this.dragging = true;
      const v = Number(this.rng.value);
      this.show(v);
      this.actions.drag(this.c.key, v);
    });
    this.rng.addEventListener("change", () => void this.commit(Number(this.rng.value)));
    this.val.addEventListener("change", () => {
      const v = parse(this.val.value);
      if (!Number.isFinite(v)) return this.fail("숫자가 아님");
      void this.commit(v);
    });
    const range = async () => {
      const lo = parse(this.min.value), hi = parse(this.max.value);
      if (!Number.isFinite(lo) || !Number.isFinite(hi)) return this.fail("범위 끝이 숫자가 아님");
      const e = await this.actions.setRange(this.c.key, lo, hi);
      if (e) this.fail(e);
    };
    this.min.addEventListener("change", range);
    this.max.addEventListener("change", range);
    for (const i of [this.val, this.min, this.max]) i.addEventListener("keydown", (e) => e.key === "Enter" && i.blur());
    this.update(c);
  }

  private async commit(v: number): Promise<void> {
    const e = await this.actions.setValue(this.c.key, v);
    this.dragging = false;
    if (e) this.fail(e);
  }

  private fail(message: string): void {
    this.err.textContent = message;
    this.err.hidden = false;
    this.update(this.c);
    setTimeout(() => (this.err.hidden = true), 4000);
  }

  private show(v: number): void {
    const c = this.c;
    if (document.activeElement !== this.val) this.val.value = fmt(v, c.step);
    const pct = c.max > c.min ? ((v - c.min) / (c.max - c.min)) * 100 : 0;
    this.fill.style.width = `${Math.min(100, Math.max(0, pct))}%`;
  }

  update(c: Constant): void {
    this.c = c;
    this.rng.min = String(c.min);
    this.rng.max = String(c.max);
    this.rng.step = String(c.step);
    if (!this.dragging) {
      this.rng.value = String(c.value);
      this.show(c.value);
    }
    if (document.activeElement !== this.min) this.min.value = fmtEnd(c.min);
    if (document.activeElement !== this.max) this.max.value = fmtEnd(c.max);
  }
}

export class Panel {
  readonly el = h("aside", { class: "panel", "aria-label": "정의와 상수" });
  private defs = h("div");
  private consts = h("div");
  private hint = h("div", { class: "hint" });
  private cards = new Map<string, ConstantCard>();
  private defKey = "";

  constructor(private actions: PanelActions) {
    this.el.append(this.hint, h("div", { class: "sec" }, "정의"), this.defs, h("div", { class: "sec" }, "상수"), this.consts);
  }

  isDragging(key: string): boolean {
    return this.cards.get(key)?.dragging ?? false;
  }

  render(state: State | null, hasKey: boolean, warnings: (key: string) => Warning[]): void {
    this.hint.textContent = !state
      ? "왼쪽에서 작업 공간을 만들거나 고르세요."
      : !hasKey
        ? "Claude가 이 작업 공간을 쓰려면 ⋯ 메뉴에서 key를 만들어 알려주세요."
        : "";
    this.hint.hidden = this.hint.textContent === "";
    this.renderDefs(state, warnings);
    this.renderConsts(state);
  }

  private renderDefs(state: State | null, warnings: (key: string) => Warning[]): void {
    const fns = state ? Object.values(state.functions).map((f) => f!).sort((a, b) => a.order - b.order) : [];
    const hidden = new Set(state ? Object.values(state.scenes).flatMap((s) => s!.hidden) : []);
    // Re-render only when something visible changed (MathJax output is cached anyway).
    const key = JSON.stringify(fns.map((f) => [f.key, f.version, f.color, hidden.has(f.key), warnings(f.key).map((w) => w.message)]));
    if (key === this.defKey) return;
    this.defKey = key;
    const ctx: LatexCtx = { colors: new Map(fns.map((f) => [f.key, colorOf(f)])) };
    this.defs.replaceChildren();
    if (state && fns.length === 0) this.defs.append(h("div", { class: "hint" }, "아직 정의가 없음. Claude가 apply로 보내면 여기에 나타남."));
    for (const f of fns) this.defs.append(this.defEl(f, ctx, !hidden.has(f.key), warnings(f.key)));
  }

  private defEl(f: Fn, ctx: LatexCtx, visible: boolean, warns: Warning[]): HTMLElement {
    const id = `d-${f.order}`;
    const box = h("input", { type: "checkbox", id, checked: visible, "aria-label": `${f.key} 보이기` });
    box.addEventListener("change", () => this.actions.toggle(f.key, box.checked));
    let math: Node;
    try {
      math = texElement(definitionTex(f.name, f.inputs, f.body, ctx));
    } catch {
      math = document.createTextNode(f.key);
    }
    return h(
      "div",
      { class: `def${visible ? "" : " off"}`, title: `${f.key}: ${f.body_src}` },
      box,
      h("div", { class: "body-col" }, h("label", { class: "math", for: id }, math), ...warns.map((w) => h("span", { class: "warning" }, icon(ICONS.warn), w.message))),
    );
  }

  private renderConsts(state: State | null): void {
    const cs = state ? Object.values(state.constants).map((c) => c!).sort((a, b) => a.order - b.order) : [];
    const keep = new Set(cs.map((c) => c.key));
    for (const k of [...this.cards.keys()]) if (!keep.has(k)) this.cards.delete(k);
    const nodes: HTMLElement[] = [];
    for (const c of cs) {
      let card = this.cards.get(c.key);
      if (!card) {
        card = new ConstantCard(c, this.actions);
        this.cards.set(c.key, card);
      } else {
        card.update(c);
      }
      nodes.push(card.el);
    }
    if (state && cs.length === 0) nodes.push(h("div", { class: "hint" }, "상수 없음"));
    this.consts.replaceChildren(...nodes);
  }
}
