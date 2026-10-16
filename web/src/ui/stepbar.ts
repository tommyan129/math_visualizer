// Step · note bar under the canvas (UI doc "step·note 바"): ◀ step n / m ▶ and the
// note of the current step. `{f}` renders f's whole definition from the current state,
// `{expr}` as math, `$...$` as given LaTeX, the rest in the UI font.

import type { NoteBlock } from "../generated/NoteBlock";
import type { Scene } from "../generated/Scene";
import type { State } from "../generated/State";
import { colorOf } from "../canvas/plot";
import { stepState } from "../canvas/scene";
import { definitionTex, exprTex, latexCtx, type LatexCtx } from "../math/latex";
import { texElement } from "../math/mathjax";
import { h, icon } from "./dom";

const PREV = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 6l-6 6 6 6"></path></svg>';
const NEXT = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6"></path></svg>';

function math(tex: string, fallback: string): Node {
  try {
    return texElement(tex);
  } catch {
    return h("code", {}, fallback);
  }
}

export function noteNodes(b: NoteBlock, state: State, ctx: LatexCtx): Node[] {
  return b.segs.map((g) => {
    switch (g.type) {
      case "Text":
        return h("span", { class: "note-text" }, g.text);
      case "Tex":
        return h("span", { class: "note-math" }, math(g.tex, g.tex));
      case "Math":
        return h("span", { class: "note-math" }, math(exprTex(g.expr, ctx), "?"));
      case "Def": {
        const f = state.functions[g.function];
        if (!f) return h("span", { class: "note-text" }, g.function);
        return h("span", { class: "note-math" }, math(definitionTex(f.name, f.inputs, f.body, ctx), f.key));
      }
    }
  });
}

export class StepBar {
  readonly el = h("div", { class: "stepbar" });
  private prev = h("button", { class: "iconbtn round", "aria-label": "이전 step" }, icon(PREV));
  private next = h("button", { class: "iconbtn round", "aria-label": "다음 step" }, icon(NEXT));
  private count = h("span", { class: "step-count" });
  private note = h("div", { class: "note" });
  private key = "";
  private at = { step: 0, last: 0 };

  constructor(go: (step: number) => void) {
    this.prev.addEventListener("click", () => this.at.step > 0 && go(this.at.step - 1));
    this.next.addEventListener("click", () => this.at.step < this.at.last && go(this.at.step + 1));
    this.el.append(h("div", { class: "stepnav" }, this.prev, this.count, this.next), this.note);
  }

  /** Steps exist in this scene. */
  get visible(): boolean {
    return !this.el.hidden;
  }

  render(state: State | null, scene: Scene | null, step: number): void {
    const last = scene ? scene.steps.length - 1 : 0;
    this.el.hidden = !state || !scene || last <= 0;
    this.at = { step, last };
    if (this.el.hidden || !state || !scene) return;
    this.prev.toggleAttribute("disabled", step <= 0);
    this.next.toggleAttribute("disabled", step >= last);
    this.count.textContent = `step ${step} / ${last}`;
    const st = stepState(scene, step);
    const blocks = st?.note ?? [];
    const fns = Object.values(state.functions).map((f) => f!);
    const versions = blocks.flatMap((b) => b.segs.map((g) => (g.type === "Def" ? state.functions[g.function]?.version ?? -1 : 0)));
    const key = JSON.stringify([scene.key, scene.version, step, versions]);
    if (key === this.key) return;
    this.key = key;
    const ctx = latexCtx(fns, colorOf);
    this.note.replaceChildren(...blocks.map((b) => h("div", { class: "note-line" }, ...noteNodes(b, state, ctx))));
    if (blocks.length === 0) this.note.append(h("span", { class: "note-empty" }, step === 0 ? "처음 상태" : ""));
  }
}
