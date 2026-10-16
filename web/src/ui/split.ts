// Draggable splitters between layout areas. Sizes are remembered per browser
// (a convenience only: without storage the defaults apply).

import { h } from "./dom";

function load(key: string): number | null {
  try {
    const v = Number(localStorage.getItem(key));
    return Number.isFinite(v) && v > 0 ? v : null;
  } catch {
    return null;
  }
}

function save(key: string, v: number): void {
  try {
    localStorage.setItem(key, String(Math.round(v)));
  } catch {
    // Storage blocked: the size just is not remembered.
  }
}

export interface SplitOptions {
  /** `x`: a vertical bar dragged left/right; `y`: a horizontal bar dragged up/down. */
  dir: "x" | "y";
  /** Storage key. */
  key: string;
  /** Size before the first drag; `null` = leave the element's own (CSS) size. */
  initial: number | null;
  min: number;
  max: () => number;
  /** +1 when dragging right/down grows the size, -1 when it shrinks it. */
  sign: 1 | -1;
  apply(size: number): void;
}

export function splitter(o: SplitOptions): HTMLElement {
  const el = h("div", { class: `split split-${o.dir}`, role: "separator", "aria-orientation": o.dir === "x" ? "vertical" : "horizontal", tabindex: 0 });
  let size = load(o.key) ?? o.initial;
  const set = (v: number, persist: boolean) => {
    size = Math.min(o.max(), Math.max(o.min, v));
    o.apply(size);
    if (persist) save(o.key, size);
  };
  if (size !== null) set(size, false);
  const measure = () => {
    const prev = el.previousElementSibling, next = el.nextElementSibling;
    const target = (o.sign === 1 ? prev : next) as HTMLElement | null;
    const r = target?.getBoundingClientRect();
    return r ? (o.dir === "x" ? r.width : r.height) : o.min;
  };
  el.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    el.setPointerCapture(e.pointerId);
    el.classList.add("active");
    const start = o.dir === "x" ? e.clientX : e.clientY;
    const from = size ?? measure();
    const move = (m: PointerEvent) => set(from + o.sign * ((o.dir === "x" ? m.clientX : m.clientY) - start), false);
    const up = () => {
      el.classList.remove("active");
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      if (size !== null) save(o.key, size);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  });
  el.addEventListener("keydown", (e) => {
    const k = e.key;
    const d = (o.dir === "x" ? (k === "ArrowRight" ? 1 : k === "ArrowLeft" ? -1 : 0) : k === "ArrowDown" ? 1 : k === "ArrowUp" ? -1 : 0) * o.sign;
    if (d === 0) return;
    e.preventDefault();
    set((size ?? measure()) + d * 16, true);
  });
  window.addEventListener("resize", () => size !== null && set(size, false));
  return el;
}
