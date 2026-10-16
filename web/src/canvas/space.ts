// Space for 2D(x, y): grid and axes as polyline + text batches (drawn like any layer).
// Grid is pressed toward the background so axes and curves read first (UI doc "캔버스").

import { rgba, type Batch, type LabelPlacement, type LabelRef, type Style } from "./primitives";
import type { View } from "./view";

export const SPACE_STYLES = {
  minor: 0,
  major: 1,
  axis: 2,
  tickLabel: 3,
  originLabel: 4,
  axisName: 5,
} as const;

export const spaceStyles: Style[] = [
  { color: rgba("#272930"), width: 1, size: 0 },
  { color: rgba("#31333b"), width: 1, size: 0 },
  { color: rgba("#858993"), width: 1.6, size: 0 },
  { color: rgba("#b8bbc3"), width: 0, size: 0 },
  { color: rgba("#858993"), width: 0, size: 0 },
  { color: rgba("#ecedf0"), width: 0, size: 0 },
];

/** Major grid spacing (1·2·5 × 10ⁿ) for at least `minPx` pixels between lines. */
export function gridStep(scale: number, minPx = 80): { major: number; minor: number; decimals: number } {
  const raw = minPx / scale;
  const p = Math.pow(10, Math.floor(Math.log10(raw)));
  const m = [1, 2, 5, 10].find((k) => k * p >= raw * 0.999) ?? 10;
  const major = m * p;
  const minor = major / (m === 5 ? 5 : 4);
  const decimals = Math.max(0, -Math.floor(Math.log10(major) + 1e-9));
  return { major, minor, decimals };
}

export function formatTick(v: number, decimals: number): string {
  const s = Math.abs(v) >= 1e6 || (Math.abs(v) < 1e-4 && v !== 0) ? v.toExponential(2) : v.toFixed(decimals);
  return s.replace("-", "−");
}

function lines(id: string, style: number, layer: 0 | 2, segs: number[]): Batch {
  const n = segs.length / 4;
  const offsets = new Uint32Array(n + 1);
  for (let i = 0; i <= n; i++) offsets[i] = i * 2;
  return { id, version: 0, source: { def: "space" }, layer, seq: style, style, origin: [0, 0, 0], dim: 2, kind: "polyline", coords: Float32Array.from(segs), offsets };
}

/** Grid lines are built in screen space around a world origin at the view center. */
export function buildSpace(view: View): Batch[] {
  const { major, minor, decimals } = gridStep(view.scale);
  const b = view.box();
  const ox = view.cx, oy = view.cy;
  const minorSegs: number[] = [], majorSegs: number[] = [];
  // Integer indices: accumulating `x += minor` drifts.
  const ratio = Math.round(major / minor);
  for (let i = Math.ceil(b.x_min / minor); i * minor <= b.x_max; i++) {
    const x = i * minor;
    (i % ratio === 0 ? majorSegs : minorSegs).push(x - ox, b.y_min - oy, x - ox, b.y_max - oy);
  }
  for (let i = Math.ceil(b.y_min / minor); i * minor <= b.y_max; i++) {
    const y = i * minor;
    (i % ratio === 0 ? majorSegs : minorSegs).push(b.x_min - ox, y - oy, b.x_max - ox, y - oy);
  }
  const grid = [lines("grid-minor", SPACE_STYLES.minor, 0, minorSegs), lines("grid-major", SPACE_STYLES.major, 0, majorSegs)];
  for (const g of grid) g.origin = [ox, oy, 0];

  // Axes through the origin; tick labels pin to the canvas edge when the axis is off screen.
  const px = 1 / view.scale;
  const xAxisY = Math.min(Math.max(0, b.y_min + 30 * px), b.y_max - 14 * px);
  const yAxisX = Math.min(Math.max(0, b.x_min + 30 * px), b.x_max - 14 * px);
  const axisSegs: number[] = [];
  const tick = 4 * px;
  const arrow = 7 * px;
  if (b.y_min <= 0 && 0 <= b.y_max) {
    axisSegs.push(b.x_min - ox, -oy, b.x_max - ox, -oy);
    const ex = b.x_max - 6 * px;
    axisSegs.push(ex - arrow - ox, arrow * 0.6 - oy, ex - ox, -oy, ex - ox, -oy, ex - arrow - ox, -arrow * 0.6 - oy);
  }
  if (b.x_min <= 0 && 0 <= b.x_max) {
    axisSegs.push(-ox, b.y_min - oy, -ox, b.y_max - oy);
    const ey = b.y_max - 6 * px;
    axisSegs.push(-arrow * 0.6 - ox, ey - arrow - oy, -ox, ey - oy, -ox, ey - oy, arrow * 0.6 - ox, ey - arrow - oy);
  }
  const labelCoords: number[] = [];
  const labels: LabelRef[] = [];
  const place: LabelPlacement[] = [];
  const styleOf: number[] = [];
  const ui = (text: string): LabelRef => ({ kind: "ui", text, px: 14, weight: 500 });
  for (let i = Math.ceil((b.x_min + 20 * px) / major); i * major <= b.x_max - 20 * px; i++) {
    const x = i * major;
    if (i === 0) continue;
    if (b.y_min <= 0 && 0 <= b.y_max) axisSegs.push(x - ox, tick - oy, x - ox, -tick - oy);
    labelCoords.push(x - ox, xAxisY - oy);
    labels.push(ui(formatTick(x, decimals)));
    place.push({ align: "center", baseline: "top", dx: 0, dy: 10, halo: true });
    styleOf.push(SPACE_STYLES.tickLabel);
  }
  for (let i = Math.ceil((b.y_min + 14 * px) / major); i * major <= b.y_max - 20 * px; i++) {
    const y = i * major;
    if (i === 0) continue;
    if (b.x_min <= 0 && 0 <= b.x_max) axisSegs.push(-tick - ox, y - oy, tick - ox, y - oy);
    labelCoords.push(yAxisX - ox, y - oy);
    labels.push(ui(formatTick(y, decimals)));
    place.push({ align: "right", baseline: "middle", dx: -12, dy: 0, halo: true });
    styleOf.push(SPACE_STYLES.tickLabel);
  }
  if (b.x_min <= 0 && 0 <= b.x_max && b.y_min <= 0 && 0 <= b.y_max) {
    labelCoords.push(-ox, -oy);
    labels.push(ui("0"));
    place.push({ align: "right", baseline: "top", dx: -12, dy: 6, halo: true });
    styleOf.push(SPACE_STYLES.originLabel);
  }
  const axes = lines("axes", SPACE_STYLES.axis, 2, axisSegs);
  axes.origin = [ox, oy, 0];
  axes.layer = 0;
  axes.seq = 10;

  const out: Batch[] = [...grid, axes];
  // One text batch per style (a batch has one style).
  for (const s of [SPACE_STYLES.tickLabel, SPACE_STYLES.originLabel]) {
    const idx = styleOf.map((st, i) => (st === s ? i : -1)).filter((i) => i >= 0);
    out.push({
      id: `ticks-${s}`,
      version: 0,
      source: { def: "space" },
      layer: 4,
      seq: s,
      style: s,
      origin: [ox, oy, 0],
      dim: 2,
      kind: "text",
      coords: Float32Array.from(idx.flatMap((i) => [labelCoords[2 * i]!, labelCoords[2 * i + 1]!])),
      labels: idx.map((i) => labels[i]!),
      placement: idx.map((i) => place[i]!),
    });
  }
  // Axis names in the math font.
  const names: number[] = [];
  const nameLabels: LabelRef[] = [];
  const namePlace: LabelPlacement[] = [];
  if (b.y_min <= 0 && 0 <= b.y_max) {
    names.push(b.x_max - 14 * px - ox, -oy);
    nameLabels.push({ kind: "tex", tex: "x", px: 22 });
    namePlace.push({ align: "right", baseline: "bottom", dx: 0, dy: -8, halo: true });
  }
  if (b.x_min <= 0 && 0 <= b.x_max) {
    names.push(-ox, b.y_max - 10 * px - oy);
    nameLabels.push({ kind: "tex", tex: "y", px: 22 });
    namePlace.push({ align: "left", baseline: "top", dx: 14, dy: 0, halo: true });
  }
  out.push({
    id: "axis-names",
    version: 0,
    source: { def: "space" },
    layer: 4,
    seq: 20,
    style: SPACE_STYLES.axisName,
    origin: [ox, oy, 0],
    dim: 2,
    kind: "text",
    coords: Float32Array.from(names),
    labels: nameLabels,
    placement: namePlace,
  });
  return out;
}
