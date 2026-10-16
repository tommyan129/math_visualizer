// Canvas 2D renderer: draws batches as given, in layer then seq order.
// It knows nothing about definitions; a WebGL2 renderer can replace it later.

import { texImage } from "../math/mathjax";
import { cssColor, POINT_HOLE, type Batch, type ClipMask, type Style } from "./primitives";
import type { View } from "./view";

const HALO = "#212329";

export class Renderer {
  private ctx: CanvasRenderingContext2D;
  /** Image batches turned into bitmaps once per id + version. */
  private images = new Map<string, { version: number; pixels: Uint8ClampedArray; canvas: HTMLCanvasElement }>();
  /** Offscreen layer for clipped batches, and the clip mask bitmap. */
  private layer: HTMLCanvasElement | null = null;
  private maskBitmap: { pixels: Uint8ClampedArray; canvas: HTMLCanvasElement } | null = null;

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

  /**
   * Draws in layer then seq order. With a `mask`, clipped batches go to an offscreen
   * layer that is cut by the mask and composited after layer 0 (grid and axes).
   */
  draw(view: View, batches: Batch[], styles: Style[], mask: ClipMask | null = null): void {
    const main = this.ctx;
    const dpr = window.devicePixelRatio || 1;
    main.setTransform(dpr, 0, 0, dpr, 0, 0);
    main.clearRect(0, 0, view.width, view.height);
    const ordered = [...batches].sort((a, b) => a.layer - b.layer || a.seq - b.seq);
    if (!mask) {
      for (const b of ordered) this.batch(view, b, styles);
    } else {
      for (const b of ordered) if (b.layer === 0 && !b.clip) this.batch(view, b, styles);
      const layer = (this.layer ??= document.createElement("canvas"));
      if (layer.width !== this.canvas.width || layer.height !== this.canvas.height) {
        layer.width = this.canvas.width;
        layer.height = this.canvas.height;
      }
      const lc = layer.getContext("2d")!;
      lc.setTransform(dpr, 0, 0, dpr, 0, 0);
      lc.clearRect(0, 0, view.width, view.height);
      this.ctx = lc;
      for (const b of ordered) if (b.clip) this.batch(view, b, styles);
      // Keep only what lies inside the mask.
      if (this.maskBitmap?.pixels !== mask.pixels) {
        const canvas = this.maskBitmap?.canvas ?? document.createElement("canvas");
        canvas.width = mask.width;
        canvas.height = mask.height;
        canvas.getContext("2d")!.putImageData(new ImageData(mask.pixels as Uint8ClampedArray<ArrayBuffer>, mask.width, mask.height), 0, 0);
        this.maskBitmap = { pixels: mask.pixels, canvas };
      }
      lc.globalCompositeOperation = "destination-in";
      lc.imageSmoothingEnabled = false;
      const r = mask.rect;
      lc.drawImage(this.maskBitmap.canvas, view.sx(r.x0), view.sy(r.y1), (r.x1 - r.x0) * view.scale, (r.y1 - r.y0) * view.scale);
      lc.globalCompositeOperation = "source-over";
      this.ctx = main;
      main.setTransform(1, 0, 0, 1, 0, 0);
      main.drawImage(layer, 0, 0);
      main.setTransform(dpr, 0, 0, dpr, 0, 0);
      for (const b of ordered) if (b.layer !== 0 && !b.clip) this.batch(view, b, styles);
    }
    const live = new Set(batches.filter((b) => b.kind === "image").map((b) => b.id));
    for (const k of [...this.images.keys()]) if (!live.has(k)) this.images.delete(k);
  }

  private batch(view: View, b: Batch, styles: Style[]): void {
    {
      const st = styles[b.style];
      if (!st) return;
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
        case "polygon":
          this.polygon(view, b, st);
          break;
        case "image":
          this.image(view, b);
          break;
      }
    }
  }

  /** The drawn canvas (snapshot). */
  element(): HTMLCanvasElement {
    return this.canvas;
  }

  private polygon(view: View, b: Extract<Batch, { kind: "polygon" }>, st: Style): void {
    const ctx = this.ctx;
    const [ox, oy] = b.origin;
    const c = b.coords;
    ctx.fillStyle = cssColor(st.color);
    for (let p = 0; p + 1 < b.polyOffsets.length; p++) {
      ctx.beginPath();
      for (let r = b.polyOffsets[p]!; r < b.polyOffsets[p + 1]!; r++) {
        const s = b.ringOffsets[r]!, e = b.ringOffsets[r + 1]!;
        for (let i = s; i < e; i++) {
          const x = view.sx(ox + c[2 * i]!), y = view.sy(oy + c[2 * i + 1]!);
          if (i === s) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.closePath();
      }
      ctx.fill("evenodd");
    }
  }

  private image(view: View, b: Extract<Batch, { kind: "image" }>): void {
    let hit = this.images.get(b.id);
    if (!hit || hit.version !== b.version || hit.pixels !== b.pixels) {
      const canvas = hit?.canvas ?? document.createElement("canvas");
      canvas.width = b.width;
      canvas.height = b.height;
      canvas.getContext("2d")!.putImageData(new ImageData(b.pixels as Uint8ClampedArray<ArrayBuffer>, b.width, b.height), 0, 0);
      hit = { version: b.version, pixels: b.pixels, canvas };
      this.images.set(b.id, hit);
    }
    const [ox, oy] = b.origin;
    const x = view.sx(ox), y = view.sy(oy);
    const w = b.size[0] * view.scale, h = b.size[1] * view.scale;
    this.ctx.imageSmoothingEnabled = true;
    this.ctx.drawImage(hit.canvas, x, y, w, h);
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
