// Primitive batches (design.md "렌더 구조"): GPU-friendly from the start.
// World coordinates as an f64 origin plus Float32Array offsets, so drag and wheel
// only change the transform. Sizes are in pixels. Style lives in a table.

export type DrawLayer = 0 | 1 | 2 | 3 | 4; // grid, fill, curve, point, label

export interface SourceRef {
  def: string;
  node?: number;
}

export interface Style {
  color: [number, number, number, number];
  /** Line width, px. */
  width: number;
  /** Point size / arrow head, px. */
  size: number;
  dash?: number[];
}

export type LabelRef =
  | { kind: "ui"; text: string; px: number; weight: number }
  | { kind: "tex"; tex: string; px: number };

export interface LabelPlacement {
  align: "left" | "center" | "right";
  baseline: "top" | "middle" | "bottom";
  /** Pixel offset from the anchor. */
  dx: number;
  dy: number;
  /** Background-colored outline so text stays readable over lines. */
  halo: boolean;
}

interface BatchBase {
  id: string;
  version: number;
  source: SourceRef;
  layer: DrawLayer;
  seq: number;
  style: number;
  origin: [number, number, number];
  dim: 2 | 3;
  /** Drawn only inside the scene clip (definitions; not the grid, labels or hover). */
  clip?: boolean;
}

/** Scene clip as a world-space alpha mask (1 = inside). */
export interface ClipMask {
  id: string;
  /** World rectangle the mask covers; outside it everything is clipped. */
  rect: { x0: number; x1: number; y0: number; y1: number };
  width: number;
  height: number;
  /** RGBA, opaque where inside. */
  pixels: Uint8ClampedArray;
}

export const POINT_NORMAL = 0;
export const POINT_HOLE = 1;
export const POINT_FEATURE = 2;

export type Batch =
  | (BatchBase & { kind: "polyline"; coords: Float32Array; offsets: Uint32Array })
  | (BatchBase & { kind: "points"; coords: Float32Array; pointKind: Uint8Array })
  | (BatchBase & { kind: "text"; coords: Float32Array; labels: LabelRef[]; placement: LabelPlacement[] })
  // Even-odd fill. `ringOffsets`: ring starts + end (points); `polyOffsets`: polygon starts + end (rings).
  | (BatchBase & { kind: "polygon"; coords: Float32Array; ringOffsets: Uint32Array; polyOffsets: Uint32Array })
  // Raster (inequality regions). `origin` is the top-left world corner; `size` is the world width / height.
  | (BatchBase & { kind: "image"; size: [number, number]; width: number; height: number; pixels: Uint8ClampedArray });

export function rgba(hex: string, alpha = 1): [number, number, number, number] {
  const n = parseInt(hex.replace("#", ""), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255, alpha];
}

export function cssColor(c: [number, number, number, number]): string {
  return `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${c[3]})`;
}
