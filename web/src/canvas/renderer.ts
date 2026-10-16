// Canvas 2D renderer: draws batches as given, in layer then seq order.
// It knows nothing about definitions; a WebGL2 renderer can replace it later.

import { texImage } from "../math/mathjax";
import { cssColor, POINT_HOLE, type Batch, type Style } from "./primitives";
import type { View } from "./view";

const HALO = "#212329";

export class Renderer {
  private ctx: CanvasRenderingContext2D;

  constructor(private canvas: HTMLCanvasElement) {
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("no 2d context");
    this.ctx = ctx;
  }

  resize(width: number, height: number): void {
    const dpr = window.devicePixelRatio || 1;
    this.canvas.width = Math.round(width * dpr);
    this.canvas.height = Math.round(height * dpr);
    this.canvas.style.width = `${width}px`;
    this.canvas.style.height = `${height}px`;
  }

  draw(view: View, batches: Batch[], styles: Style[]): void {
    const ctx = this.ctx;
    const dpr = window.devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, view.width, view.height);
    const ordered = [...batches].sort((a, b) => a.layer - b.layer || a.seq - b.seq);
    for (const b of ordered) {
      const st = styles[b.style];
      if (!st) continue;
      switch (b.kind) {
        case "polyline":
          this.polyline(view, b, st);
          break;
        case "points":
          this.points(view, b, st);
          break;
        case "text":
          this.text(view, b, st);
          break;
      }
    }
  }

  private polyline(view: View, b: Extract<Batch, { kind: "polyline" }>, st: Style): void {
    const ctx = this.ctx;
    const [ox, oy] = b.origin;
    ctx.strokeStyle = cssColor(st.color);
    ctx.lineWidth = st.width;
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    ctx.setLineDash(st.dash ?? []);
    ctx.beginPath();
    const c = b.coords;
    for (let p = 0; p + 1 < b.offsets.length; p++) {
      const s = b.offsets[p]!, e = b.offsets[p + 1]!;
      for (let i = s; i < e; i++) {
        const x = view.sx(ox + c[2 * i]!), y = view.sy(oy + c[2 * i + 1]!);
        if (i === s) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
    }
    ctx.stroke();
  }

  private points(view: View, b: Extract<Batch, { kind: "points" }>, st: Style): void {
    const ctx = this.ctx;
    const [ox, oy] = b.origin;
    const color = cssColor(st.color);
    const r = st.size / 2;
    for (let i = 0; i < b.pointKind.length; i++) {
      const x = view.sx(ox + b.coords[2 * i]!), y = view.sy(oy + b.coords[2 * i + 1]!);
      if (x < -r || y < -r || x > view.width + r || y > view.height + r) continue;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      if (b.pointKind[i] === POINT_HOLE) {
        ctx.fillStyle = HALO;
        ctx.fill();
        ctx.strokeStyle = color;
        ctx.lineWidth = 2.5;
        ctx.stroke();
      } else {
        ctx.fillStyle = color;
        ctx.fill();
      }
    }
  }

  private text(view: View, b: Extract<Batch, { kind: "text" }>, st: Style): void {
    const ctx = this.ctx;
    const [ox, oy] = b.origin;
    const color = cssColor(st.color);
    b.labels.forEach((label, i) => {
      const pl = b.placement[i]!;
      const x = view.sx(ox + b.coords[2 * i]!) + pl.dx;
      const y = view.sy(oy + b.coords[2 * i + 1]!) + pl.dy;
      if (label.kind === "ui") {
        ctx.font = `${label.weight} ${label.px}px SUIT, sans-serif`;
        ctx.textAlign = pl.align;
        ctx.textBaseline = pl.baseline === "bottom" ? "alphabetic" : pl.baseline;
        if (pl.halo) {
          ctx.strokeStyle = HALO;
          ctx.lineWidth = 5;
          ctx.lineJoin = "round";
          ctx.setLineDash([]);
          ctx.strokeText(label.text, x, y);
        }
        ctx.fillStyle = color;
        ctx.fillText(label.text, x, y);
        return;
      }
      const dpr = window.devicePixelRatio || 1;
      const img = texImage(label.tex, color, label.px, HALO, dpr);
      if (!img.ready) return;
      let left = pl.align === "left" ? x : pl.align === "center" ? x - img.width / 2 : x - img.width;
      let top = pl.baseline === "top" ? y : pl.baseline === "middle" ? y - img.height / 2 : y - img.height;
      // Snap to device pixels so the bitmap is not resampled.
      left = Math.round(left * dpr) / dpr;
      top = Math.round(top * dpr) / dpr;
      if (pl.halo) ctx.drawImage(img.halo, left, top, img.width, img.height);
      ctx.drawImage(img.img, left, top, img.width, img.height);
    });
  }
}
