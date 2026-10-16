// Definition and constant panel (UI doc "정의·토글 패널", "상수 슬라이더").
// Definitions: checkbox + expression (function name in its graph color), warnings below.
// Constants: one card each; drag updates locally, release sends the final value.

import type { Constant } from "../generated/Constant";
import type { Function as Fn } from "../generated/Function";
import type { Highlight } from "../generated/Highlight";
import type { Scene } from "../generated/Scene";
import type { State } from "../generated/State";
import type { Warning } from "../generated/Warning";
import { resolveHighlights, sortedHighlights } from "../canvas/highlight";
import { colorOf } from "../canvas/plot";
import { activeClip, concealedAt, visibleAt } from "../canvas/scene";
import { definitionTex, exprTex, identTex, type LatexCtx } from "../math/latex";
import { texElement } from "../math/mathjax";
import { h, icon, ICONS } from "./dom";

export interface PanelActions {
  /** Show / hide a function or highlight. */
  toggle(key: string, visible: boolean): void;
  /** `del key` through the DSL path. */
  remove(key: string): void;
  drag(constant: string, value: number): void;
  /** Resolves to an error message, or null. */
  setValue(constant: string, value: number): Promise<string | null>;
  setRange(constant: string, min: number, max: number): Promise<string | null>;
  /** The user's clip as DSL text; null = back to Claude's. Resolves to an error or null. */
  setClip(src: string | null): Promise<string | null>;
}

/** What the panel shows: the current scene and step of the open workspace. */
export interface PanelView {
  state: State | null;
  scene: Scene | null;
  step: number;
  hasKey: boolean;
  warnings(key: string): Warning[];
  sceneWarnings: Warning[];
  /** How a span is drawn now: 0 point, 1 line, 2 plane; undefined before it is drawn. */
  spanDim(key: string): number | undefined;
}

const SPAN_DIM = ["점 (원점)", "직선", "평면"];

/** Vectors, matrices and matrix functions: listed in every scene, never toggled. */
const drawn = (f: Fn) => f.kind === "Curve" || f.kind === "Span" || f.kind === "Arrow";

function hexAlpha(hex: string, a: number): string {
  const n = parseInt(hex.replace("#", ""), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

/** Width of the slider thumb (`.rng::-webkit-slider-thumb`). */
const THUMB_PX = 18;

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
    this.rng = h("input", { class: "rng", type: "range", id, title: "Shift를 누른 채 끌면 한 칸(step)씩" });
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
    this.pointerDrag();
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

  /**
   * Mouse / touch drag, replacing the native one (keyboard stays native). The drag is
   * relative: normally 1px follows the cursor, with Shift held 1px is one step, so
   * every step value can be reached (a native slider moves 2–3 steps per pixel).
   */
  private pointerDrag(): void {
    let raw = 0, lastX = 0, start = 0, sent = 0;
    const pxRate = (shift: boolean) => {
      const c = this.c;
      return shift ? c.step : (c.max - c.min) / Math.max(1, this.rng.getBoundingClientRect().width - THUMB_PX);
    };
    const emit = () => {
      const c = this.c;
      raw = Math.min(c.max, Math.max(c.min, raw));
      const v = Math.min(c.max, Number((c.min + Math.round((raw - c.min) / c.step) * c.step).toFixed(12)));
      if (v === sent) return;
      sent = v;
      this.rng.value = String(v);
      this.show(v);
      this.actions.drag(this.c.key, v);
    };
    this.rng.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      this.rng.focus();
      this.rng.setPointerCapture(e.pointerId);
      this.dragging = true;
      start = sent = Number(this.rng.value);
      raw = start;
      lastX = e.clientX;
      // Plain click on the track jumps to the cursor; Shift keeps the value.
      if (!e.shiftKey) {
        const r = this.rng.getBoundingClientRect();
        raw = this.c.min + (e.clientX - r.left - THUMB_PX / 2) * pxRate(false);
        emit();
      }
    });
    this.rng.addEventListener("pointermove", (e) => {
      if (!this.rng.hasPointerCapture(e.pointerId)) return;
      raw += (e.clientX - lastX) * pxRate(e.shiftKey);
      lastX = e.clientX;
      emit();
    });
    const end = (e: PointerEvent) => {
      if (!this.rng.hasPointerCapture(e.pointerId)) return;
      this.rng.releasePointerCapture(e.pointerId);
      if (sent !== start) void this.commit(sent);
      else this.dragging = false;
    };
    this.rng.addEventListener("pointerup", end);
    this.rng.addEventListener("pointercancel", end);
  }

  private async commit(v: number): Promise<void> {
    this.previewing = false;
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

  /** Animate playback is moving this slider: snapshots do not reset it. */
  previewing = false;

  get constant(): Constant {
    return this.c;
  }

  /** Moves the slider without sending (animate playback). */
  preview(v: number): void {
    this.previewing = true;
    this.rng.value = String(v);
    this.show(v);
  }

  update(c: Constant): void {
    this.c = c;
    this.rng.min = String(c.min);
    this.rng.max = String(c.max);
    this.rng.step = String(c.step);
    if (!this.dragging && !this.previewing) {
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
  private hlSec = h("div", { class: "sec" }, "강조");
  private hls = h("div");
  private hlKey = "";
  /** Highlight memos the user expanded. */
  private open = new Set<string>();
  private consts = h("div");
  private hint = h("div", { class: "hint" });
  private cards = new Map<string, ConstantCard>();
  private defKey = "";
  private clipSec = h("div", { class: "sec" });
  private clipBox = h("div", { class: "clip-box" });
  private clipInput = h("input", { class: "clip-input", "aria-label": "clip", spellcheck: "false" });
  private clipReset = h("button", { class: "iconbtn small", "aria-label": "Claude의 clip으로", title: "Claude의 clip으로" }, icon(ICONS.reset));
  private clipErr = h("div", { class: "err", hidden: true });
  private sceneWarn = h("div", { class: "scene-warn" });
  private others = h("div", { class: "hint others" });
  private clipSrc = "";
  private v: PanelView = { state: null, scene: null, step: 0, hasKey: false, warnings: () => [], sceneWarnings: [], spanDim: () => undefined };

  constructor(private actions: PanelActions) {
    this.clipBox.append(this.clipInput, this.clipReset);
    this.el.append(this.hint, this.sceneWarn, h("div", { class: "sec" }, "정의"), this.defs, this.others, this.hlSec, this.hls, h("div", { class: "sec" }, "상수"), this.consts, this.clipSec, this.clipBox, this.clipErr);
    const send = async (src: string | null) => {
      const e = await this.actions.setClip(src);
      this.clipErr.textContent = e ?? "";
      this.clipErr.hidden = !e;
      if (e) this.clipInput.value = this.clipSrc;
    };
    this.clipInput.addEventListener("change", () => {
      const t = this.clipInput.value.trim();
      if (t === this.clipSrc) return;
      void send(t === "" ? null : t);
    });
    this.clipInput.addEventListener("keydown", (e) => e.key === "Enter" && this.clipInput.blur());
    this.clipReset.addEventListener("click", () => void send(null));
  }

  isDragging(key: string): boolean {
    return this.cards.get(key)?.dragging ?? false;
  }

  /** Slider shows a value set from outside (animate) without sending it, until `endPreview`. */
  showValue(key: string, value: number): void {
    this.cards.get(key)?.preview(value);
  }

  endPreview(key: string): void {
    const c = this.cards.get(key);
    if (!c) return;
    c.previewing = false;
    c.update(c.constant);
  }

  /** Is `key` listed (a member of the scene, not concealed)? */
  private listed(key: string): boolean {
    const sc = this.v.scene;
    return !!sc && sc.members.includes(key) && !concealedAt(sc, this.v.step).has(key);
  }

  private shown(key: string): boolean {
    return !!this.v.scene && visibleAt(this.v.scene, this.v.step, key);
  }

  render(v: PanelView): void {
    this.v = v;
    const { state, hasKey } = v;
    this.render0 = () => this.renderHighlights(v.state, v.warnings);
    this.hint.textContent = !state
      ? "왼쪽에서 작업 공간을 만들거나 고르세요."
      : !hasKey
        ? "Claude가 이 작업 공간을 쓰려면 ⋯ 메뉴에서 key를 만들어 알려주세요."
        : "";
    this.hint.hidden = this.hint.textContent === "";
    this.sceneWarn.replaceChildren(...v.sceneWarnings.map((w) => h("span", { class: "warning" }, icon(ICONS.warn), w.message)));
    this.sceneWarn.hidden = v.sceneWarnings.length === 0;
    this.renderDefs(state, v.warnings);
    this.renderHighlights(state, v.warnings);
    this.renderConsts(state);
    this.renderClip();
  }

  private renderClip(): void {
    const sc = this.v.scene;
    const show = !!sc;
    this.clipSec.hidden = this.clipBox.hidden = !show;
    if (!sc) {
      this.clipErr.hidden = true;
      return;
    }
    this.clipSec.textContent = `clip · ${sc.number === null ? sc.key : `scene ${sc.number}`}`;
    const c = activeClip(sc);
    this.clipSrc = c?.src ?? "";
    if (document.activeElement !== this.clipInput) this.clipInput.value = this.clipSrc;
    this.clipInput.placeholder = "clip 없음 (예: 0 <= x <= 10)";
    this.clipReset.hidden = !sc.user_clip;
  }

  private renderHighlights(state: State | null, warnings: (key: string) => Warning[]): void {
    const hs = state ? sortedHighlights(state).filter((x) => this.listed(x.key)) : [];
    const hidden = new Set(hs.filter((x) => !this.shown(x.key)).map((x) => x.key));
    const key = JSON.stringify([this.v.scene?.key, hs.map((x) => [x.key, x.version, hidden.has(x.key), warnings(x.key).map((w) => w.message)]), [...this.open]]);
    if (key === this.hlKey) return;
    this.hlKey = key;
    this.hlSec.hidden = hs.length === 0;
    const fns = state ? Object.values(state.functions).map((f) => f!) : [];
    const ctx: LatexCtx = { colors: new Map(fns.map((f) => [f.key, colorOf(f)])) };
    this.hls.replaceChildren(...hs.map((x) => this.hlEl(x, state!, ctx, !hidden.has(x.key), warnings(x.key))));
  }

  private hlEl(x: Highlight, state: State, ctx: LatexCtx, visible: boolean, warns: Warning[]): HTMLElement {
    const id = `h-${x.order}`;
    const box = h("input", { type: "checkbox", id, checked: visible, "aria-label": `${x.key} 보이기` });
    box.addEventListener("change", () => this.actions.toggle(x.key, box.checked));
    const sub = x.target.type === "Sub";
    const chip = h("span", { class: `chip${sub ? " sub" : ""}` });
    chip.style.background = sub ? x.style.color : hexAlpha(x.style.color, Math.min(1, x.style.opacity * 2.5));
    const parts: Node[] = [];
    const t = x.target;
    if (t.type === "Function") {
      const f = state.functions[t.function];
      if (f) parts.push(texElement(identTex(f.name, colorOf(f))));
    } else if (t.type === "Sub") {
      parts.push(h("code", {}, t.address));
    }
    if (x.source.range) {
      if (parts.length) parts.push(document.createTextNode(", "));
      try {
        parts.push(texElement(exprTex(x.source.range, ctx)));
      } catch {
        parts.push(h("code", {}, x.src));
      }
    }
    const del = h("button", { class: "iconbtn small del", "aria-label": `${x.key} 지우기`, title: `del ${x.key}`, onclick: () => this.actions.remove(x.key) }, icon(ICONS.close));
    const memo = x.memo
      ? h("button", { class: `memo${this.open.has(x.key) ? " open" : ""}`, title: x.memo, onclick: () => {
          if (this.open.has(x.key)) this.open.delete(x.key);
          else this.open.add(x.key);
          this.hlKey = "";
          this.render0();
        } }, x.memo)
      : null;
    return h(
      "div",
      { class: `def hl${visible ? "" : " off"}`, title: `${x.key} = ${x.src}` },
      box,
      h(
        "div",
        { class: "body-col" },
        h("label", { class: "hl-head", for: id }, chip, h("span", { class: "hl-name" }, x.key), h("span", { class: "hl-what" }, ...parts)),
        memo,
        ...warns.map((w) => h("span", { class: "warning" }, icon(ICONS.warn), w.message)),
      ),
      del,
    );
  }

  /** Re-render after a local UI change (memo expand); set by `render`. */
  private render0: () => void = () => {};

  private renderDefs(state: State | null, warnings: (key: string) => Warning[]): void {
    const all = state ? Object.values(state.functions).map((f) => f!).sort((a, b) => a.order - b.order) : [];
    const fns = all.filter((f) => !drawn(f) || this.listed(f.key));
    const hidden = new Set(state ? [...Object.keys(state.functions), ...Object.keys(state.highlights)].filter((k) => !this.shown(k)) : []);
    const elsewhere = state ? [...all.filter(drawn).map((f) => f.key), ...Object.keys(state.highlights)].filter((k) => !this.v.scene?.members.includes(k)).length : 0;
    this.others.textContent = elsewhere > 0 ? `다른 좌표계 정의 ${elsewhere}개` : "";
    this.others.hidden = elsewhere === 0 || !this.v.scene;
    // Re-render only when something visible changed (MathJax output is cached anyway).
    const nodes = state ? resolveHighlights(state, hidden).nodeColors : new Map<string, Map<number, string>>();
    const key = JSON.stringify([this.v.scene?.key, fns.map((f) => [f.key, f.version, f.color, hidden.has(f.key), warnings(f.key).map((w) => w.message), [...(nodes.get(f.key) ?? [])], this.v.spanDim(f.key)])]);
    if (key === this.defKey) return;
    this.defKey = key;
    const colors = new Map(all.map((f) => [f.key, colorOf(f)]));
    this.defs.replaceChildren();
    if (state && all.length === 0) this.defs.append(h("div", { class: "hint" }, "아직 정의가 없음. Claude가 apply로 보내면 여기에 나타남."));
    for (const f of fns) this.defs.append(this.defEl(f, { colors, nodes: nodes.get(f.key) }, !drawn(f) || !hidden.has(f.key), warnings(f.key)));
  }

  private defEl(f: Fn, ctx: LatexCtx, visible: boolean, warns: Warning[]): HTMLElement {
    const id = `d-${f.order}`;
    let box: HTMLElement;
    if (drawn(f)) {
      const input = h("input", { type: "checkbox", id, checked: visible, "aria-label": `${f.key} 보이기` });
      input.addEventListener("change", () => this.actions.toggle(f.key, input.checked));
      box = input;
    } else {
      // Keeps the math aligned with the toggled rows.
      box = h("span", { class: "nobox", title: "다른 정의에서 쓰는 값 (그리지 않음)" });
    }
    const dim = f.kind === "Span" && visible ? this.v.spanDim(f.key) : undefined;
    const badge = dim !== undefined ? h("span", { class: "span-dim" }, `${SPAN_DIM[dim] ?? `${dim}차원`} · ${dim}차원`) : null;
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
      h("div", { class: "body-col" }, h("label", { class: "math", for: id }, math), badge, ...warns.map((w) => h("span", { class: "warning" }, icon(ICONS.warn), w.message))),
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
    // Re-inserting a card being dragged would end the drag (pointer capture is lost).
    const same = nodes.length === this.consts.children.length && nodes.every((n, i) => this.consts.children[i] === n);
    if (!same) this.consts.replaceChildren(...nodes);
  }
}
