// Modal dialog over the app: title, close button, Escape / backdrop click closes.

import { h, icon, ICONS } from "./dom";

export interface Modal {
  body: HTMLElement;
  close(): void;
}

export function openModal(title: string, onClose?: () => void): Modal {
  const body = h("div", { class: "modal-body" });
  const box = h("div", { class: "modal", role: "dialog", "aria-modal": "true", "aria-label": title });
  const back = h("div", { class: "modal-back" }, box);
  const close = () => {
    back.remove();
    document.removeEventListener("keydown", onKey, true);
    onClose?.();
  };
  const onKey = (e: KeyboardEvent) => {
    // A key capture in the dialog handles Escape itself.
    if (e.key === "Escape" && !box.querySelector(".capturing")) {
      e.preventDefault();
      close();
    }
  };
  box.append(h("div", { class: "modal-head" }, h("span", { class: "modal-title" }, title), h("button", { class: "iconbtn", "aria-label": "닫기", onclick: close }, icon(ICONS.close))), body);
  back.addEventListener("pointerdown", (e) => e.target === back && close());
  document.addEventListener("keydown", onKey, true);
  document.body.append(back);
  return { body, close };
}
