// User shortcuts (settings screen). Kept per browser in localStorage.
// A binding is a keyboard code (`KeyW`, `Space`, `ArrowRight`) or a mouse button
// (`Mouse0` left, `Mouse1` middle, `Mouse2` right). Every action has two slots.
// "Hold" actions act while the binding is down (moving, panning, turning: a key held
// while the mouse moves, or a mouse button dragged); "press" actions fire once.

export type Action = "pan" | "turn" | "forward" | "back" | "left" | "right" | "up" | "down" | "speed" | "stepNext" | "stepPrev";

export interface ActionInfo {
  action: Action;
  group: string;
  label: string;
  /** Mouse buttons allowed (pan / turn: drag; others are keys). */
  mouse: boolean;
  defaults: [string | null, string | null];
}

export const ACTIONS: ActionInfo[] = [
  { action: "pan", group: "3D 화면", label: "화면 이동 (누른 채 끌기)", mouse: true, defaults: ["Mouse0", null] },
  { action: "turn", group: "3D 화면", label: "시선 돌리기 (누른 채 끌기)", mouse: true, defaults: ["Mouse2", null] },
  { action: "forward", group: "3D 화면", label: "앞으로", mouse: false, defaults: ["KeyW", null] },
  { action: "back", group: "3D 화면", label: "뒤로", mouse: false, defaults: ["KeyS", null] },
  { action: "left", group: "3D 화면", label: "왼쪽", mouse: false, defaults: ["KeyA", null] },
  { action: "right", group: "3D 화면", label: "오른쪽", mouse: false, defaults: ["KeyD", null] },
  { action: "up", group: "3D 화면", label: "위로", mouse: false, defaults: ["Space", null] },
  { action: "down", group: "3D 화면", label: "아래로", mouse: false, defaults: ["KeyQ", null] },
  { action: "speed", group: "3D 화면", label: "이동 속도 ×1 → ×2 → ×5 (누를 때마다)", mouse: false, defaults: ["ShiftLeft", "ShiftRight"] },
  { action: "stepNext", group: "step", label: "다음 step", mouse: false, defaults: ["ArrowRight", "PageDown"] },
  { action: "stepPrev", group: "step", label: "이전 step", mouse: false, defaults: ["ArrowLeft", "PageUp"] },
];

/** Fixed, shown in the settings list for reference. */
export const FIXED: { group: string; label: string; binding: string }[] = [
  { group: "화면", label: "확대·축소 (2D), 가리킨 곳으로 다가가기 (3D)", binding: "휠" },
  { group: "2D 화면", label: "화면 이동", binding: "왼쪽 끌기" },
];

/** Keys the browser keeps for itself (Ctrl / Meta combinations are never bound). */
const RESERVED = new Set(["Escape", "Tab", "F5", "F11", "F12"]);

type Map = Record<Action, [string | null, string | null]>;

const STORE = "mv.keymap";
let map: Map = load();
const listeners: (() => void)[] = [];

function defaults(): Map {
  return Object.fromEntries(ACTIONS.map((a) => [a.action, [...a.defaults]])) as Map;
}

function load(): Map {
  const m = defaults();
  try {
    const saved = JSON.parse(localStorage.getItem(STORE) ?? "null") as Partial<Map> | null;
    // Saved before the speed toggle took Shift: "down" still on its old default moves to Q.
    if (saved && !saved.speed && saved.down?.[0] === "ShiftLeft" && saved.down?.[1] === "ShiftRight") delete saved.down;
    if (saved) for (const a of ACTIONS) if (Array.isArray(saved[a.action])) m[a.action] = [saved[a.action]![0] ?? null, saved[a.action]![1] ?? null];
  } catch {
    // Defaults.
  }
  return m;
}

function save(): void {
  try {
    localStorage.setItem(STORE, JSON.stringify(map));
  } catch {
    // Kept for this page only.
  }
  listeners.forEach((f) => f());
}

export function onKeymapChange(f: () => void): void {
  listeners.push(f);
}

export function bindings(a: Action): [string | null, string | null] {
  return map[a];
}

/** Is `code` (a key code or `MouseN`) bound to `a`? */
export function bound(a: Action, code: string): boolean {
  return map[a].includes(code);
}

/** Every action `code` is bound to. */
export function actionsOf(code: string): Action[] {
  return ACTIONS.filter((a) => map[a.action].includes(code)).map((a) => a.action);
}

/**
 * Sets one slot. A binding used elsewhere moves here (its old slot is cleared); the
 * returned label names the action it was taken from.
 */
export function setBinding(a: Action, slot: 0 | 1, code: string | null): string | null {
  let moved: string | null = null;
  if (code) {
    for (const other of ACTIONS) {
      const b = map[other.action];
      for (const i of [0, 1] as const) {
        if (b[i] === code && !(other.action === a && i === slot)) {
          b[i] = null;
          moved = other.label;
        }
      }
    }
  }
  map[a][slot] = code;
  save();
  return moved;
}

export function resetKeymap(): void {
  map = defaults();
  save();
}

/** Can this code be bound to the action? */
export function allowed(a: Action, code: string): boolean {
  if (RESERVED.has(code)) return false;
  const info = ACTIONS.find((x) => x.action === a)!;
  return code.startsWith("Mouse") ? info.mouse : true;
}

const NAMES: Record<string, string> = {
  Mouse0: "왼쪽 클릭",
  Mouse1: "가운데 클릭",
  Mouse2: "오른쪽 클릭",
  Space: "Space",
  ShiftLeft: "왼쪽 Shift",
  ShiftRight: "오른쪽 Shift",
  ControlLeft: "왼쪽 Ctrl",
  ControlRight: "오른쪽 Ctrl",
  AltLeft: "왼쪽 Alt",
  AltRight: "오른쪽 Alt",
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Enter: "Enter",
  Backspace: "Backspace",
};

/** Display name of a binding. */
export function bindingName(code: string): string {
  if (NAMES[code]) return NAMES[code]!;
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (code.startsWith("Numpad")) return `숫자패드 ${code.slice(6)}`;
  return code;
}

/** Typing into a field: shortcuts stay out of the way. */
export function typing(e: Event): boolean {
  const t = e.target as HTMLElement | null;
  return !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);
}
