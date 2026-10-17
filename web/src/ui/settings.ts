// Settings screen (sidebar ⚙): shortcuts. Click a slot, then press a key or mouse
// button; Backspace clears it, Escape cancels. A binding taken from another action
// leaves that slot empty (shown in a note).

import { ACTIONS, allowed, bindingName, bindings, FIXED, resetKeymap, setBinding, type Action } from "./keymap";
import { h } from "./dom";
import { openModal } from "./modal";

export function openSettings(): void {
  const m = openModal("설정");
  let note = "";
  let stop: (() => void) | null = null;
  const render = () => {
    m.body.replaceChildren();
    m.body.append(h("div", { class: "set-section" }, "단축키"), h("p", { class: "set-hint" }, "칸을 누른 뒤 키나 마우스 버튼을 누르면 바뀜 · Backspace 지우기 · Esc 취소. 3D 화면 조작은 마우스가 3D 화면 위에 있을 때만 동작함."));
    const table = h("div", { class: "keys" });
    let group = "";
    for (const a of ACTIONS) {
      if (a.group !== group) {
        group = a.group;
        table.append(h("div", { class: "keys-group" }, group));
      }
      const b = bindings(a.action);
      table.append(h("div", { class: "keys-row" }, h("span", { class: "keys-label" }, a.label), slot(a.action, 0, b[0]), slot(a.action, 1, b[1])));
    }
    for (const f of FIXED) table.append(h("div", { class: "keys-row fixed" }, h("span", { class: "keys-label" }, `${f.label} (${f.group})`), h("span", { class: "keys-slot fixed" }, f.binding), h("span", {})));
    m.body.append(table);
    if (note) m.body.append(h("p", { class: "set-note" }, note));
    m.body.append(
      h(
        "div",
        { class: "set-actions" },
        h(
          "button",
          {
            class: "ghost-btn",
            onclick: () => {
              resetKeymap();
              note = "기본값으로 되돌림";
              render();
            },
          },
          "기본값으로",
        ),
      ),
    );
  };

  const slot = (action: Action, i: 0 | 1, code: string | null): HTMLElement => {
    const el = h("button", { class: `keys-slot${code ? "" : " unset"}` }, code ? bindingName(code) : "—");
    el.addEventListener("click", () => capture(action, i, el));
    return el;
  };

  const capture = (action: Action, i: 0 | 1, el: HTMLElement) => {
    stop?.();
    el.textContent = "누르세요…";
    el.classList.add("capturing");
    const done = (code: string | null | undefined) => {
      cleanup();
      if (code !== undefined) {
        const from = setBinding(action, i, code);
        note = from ? `${bindingName(code!)}: '${from}'에서 옮김 (그 칸은 비움)` : "";
      }
      render();
    };
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") return done(undefined);
      if (e.code === "Backspace" || e.code === "Delete") return done(null);
      if (e.ctrlKey || e.metaKey || e.altKey) {
        note = "Ctrl·Alt·Cmd 조합은 브라우저 단축키와 겹쳐서 쓸 수 없음";
        return done(undefined);
      }
      if (!allowed(action, e.code)) {
        note = `${bindingName(e.code)}는 이 동작에 쓸 수 없음`;
        return done(undefined);
      }
      done(e.code);
    };
    const onMouse = (e: PointerEvent) => {
      const code = `Mouse${e.button}`;
      e.preventDefault();
      e.stopPropagation();
      if (!allowed(action, code)) {
        // Clicking elsewhere with a key-only action: cancel.
        return done(undefined);
      }
      done(code);
    };
    const noMenu = (e: Event) => e.preventDefault();
    const cleanup = () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("pointerdown", onMouse, true);
      window.removeEventListener("contextmenu", noMenu, true);
      stop = null;
    };
    stop = () => {
      cleanup();
      render();
    };
    window.addEventListener("keydown", onKey, true);
    // After this click finishes, so it is not taken as the binding.
    setTimeout(() => {
      if (stop) {
        window.addEventListener("pointerdown", onMouse, true);
        window.addEventListener("contextmenu", noMenu, true);
      }
    }, 0);
  };

  render();
}
