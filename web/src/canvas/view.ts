// World ↔ screen transform for 2D(x, y). Equal scale on both axes (1:1).

import type { ViewBox } from "../generated/ViewBox";

/** Default view: x from -10 to 10 (design.md "view 생략 시 고정 기본 범위"). */
export const DEFAULT_HALF_WIDTH = 10;

export class View {
  /** World point at the canvas center. */
  cx = 0;
  cy = 0;
  /** CSS pixels per world unit. */
  scale = 40;
  width = 0;
  height = 0;

  resize(width: number, height: number): void {
    const first = this.width === 0;
    this.width = width;
    this.height = height;
    if (first) this.reset();
  }

  reset(): void {
    this.cx = 0;
    this.cy = 0;
    this.scale = this.width > 0 ? this.width / (2 * DEFAULT_HALF_WIDTH) : 40;
  }

  sx(x: number): number {
    return this.width / 2 + (x - this.cx) * this.scale;
  }

  sy(y: number): number {
    return this.height / 2 - (y - this.cy) * this.scale;
  }

  wx(px: number): number {
    return this.cx + (px - this.width / 2) / this.scale;
  }

  wy(py: number): number {
    return this.cy - (py - this.height / 2) / this.scale;
  }

  box(): ViewBox {
    return { x_min: this.wx(0), x_max: this.wx(this.width), y_min: this.wy(this.height), y_max: this.wy(0), z_min: null, z_max: null };
  }

  panBy(dxPx: number, dyPx: number): void {
    this.cx -= dxPx / this.scale;
    this.cy += dyPx / this.scale;
  }

  /** Zooms keeping the world point under (px, py) fixed. */
  zoomAt(px: number, py: number, factor: number): void {
    const x = this.wx(px), y = this.wy(py);
    this.scale = Math.min(1e9, Math.max(1e-6, this.scale * factor));
    this.cx = x - (px - this.width / 2) / this.scale;
    this.cy = y + (py - this.height / 2) / this.scale;
  }
}
