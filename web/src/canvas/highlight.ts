// Highlight overlap (design.md "겹침 규칙"), resolved before any Primitive is made:
// - fills each keep their own color (fixed at creation) and are drawn translucent in
//   creation order, so overlapping fills blend;
// - every other property is last-wins per property: for an expression node colored by
//   several highlights, the latest one's color is used.
// Each highlight stays its own object; hiding or deleting a later one shows the earlier.

import type { Highlight } from "../generated/Highlight";
import type { State } from "../generated/State";

export interface HighlightLayer {
  /** Visible function / targetless highlights, in creation order. */
  fills: Highlight[];
  /** Per function key: node id → CSS color for the expression (last wins). */
  nodeColors: Map<string, Map<number, string>>;
}

export function sortedHighlights(state: State): Highlight[] {
  return Object.values(state.highlights)
    .map((h) => h!)
    .sort((a, b) => a.order - b.order);
}

export function resolveHighlights(state: State, hidden: Set<string>): HighlightLayer {
  const fills: Highlight[] = [];
  const nodeColors = new Map<string, Map<number, string>>();
  for (const h of sortedHighlights(state)) {
    if (hidden.has(h.key)) continue;
    const t = h.target;
    if (t.type === "Sub") {
      let m = nodeColors.get(t.function);
      if (!m) nodeColors.set(t.function, (m = new Map()));
      for (const n of t.nodes) m.set(n, h.style.color);
    } else {
      fills.push(h);
    }
  }
  return { fills, nodeColors };
}

/** RGBA pixels for a raster mask in one color. */
export function maskPixels(mask: Uint8Array, hex: string, opacity: number): Uint8ClampedArray {
  const n = parseInt(hex.replace("#", ""), 16);
  const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255, a = Math.round(opacity * 255);
  const px = new Uint8ClampedArray(mask.length * 4);
  for (let i = 0; i < mask.length; i++) {
    if (!mask[i]) continue;
    px[4 * i] = r;
    px[4 * i + 1] = g;
    px[4 * i + 2] = b;
    px[4 * i + 3] = a;
  }
  return px;
}
