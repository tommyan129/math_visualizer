// MathJax SVG output with the TeX font (design.md "수식 렌더"). The look matches
// claude.ai's LaTeX rendering (TeX font family), not MathJax 4's default font.
// Labels are rendered once per string and cached as images for the canvas.

import { mathjax } from "@mathjax/src/js/mathjax.js";
import { TeX } from "@mathjax/src/js/input/tex.js";
import { SVG } from "@mathjax/src/js/output/svg.js";
import { browserAdaptor } from "@mathjax/src/js/adaptors/browserAdaptor.js";
import { RegisterHTMLHandler } from "@mathjax/src/js/handlers/html.js";
import "@mathjax/src/js/input/tex/base/BaseConfiguration.js";
import "@mathjax/src/js/input/tex/ams/AmsConfiguration.js";
import "@mathjax/src/js/input/tex/color/ColorConfiguration.js";
import { MathJaxTexFont } from "@mathjax/mathjax-tex-font/js/svg.js";
import { defineSha, SHA_PACKAGE } from "./sha";

const adaptor = browserAdaptor();
RegisterHTMLHandler(adaptor);
// No shared glyph cache: every SVG must stand alone (DOM copies and canvas images).
// No inline line breaking: MathJax 4 would split one expression into several SVGs.
const output = new SVG({ fontData: MathJaxTexFont, fontCache: "none", linebreaks: { inline: false } });
defineSha(output.font);
const doc = mathjax.document(document, {
  InputJax: new TeX({ packages: ["base", "ams", "color", SHA_PACKAGE] }),
  OutputJax: output,
});

/** TeX ex height relative to em. */
const EX_PER_EM = 0.430554;

/** Renders inline TeX to a standalone SVG element sized in `ex` (scales with CSS font-size). */
export function texToSvg(tex: string): SVGSVGElement {
  const container = doc.convert(tex, { display: false, em: 16, ex: 16 * EX_PER_EM }) as HTMLElement;
  const svg = container.querySelector("svg");
  if (!svg) throw new Error("MathJax produced no SVG");
  return svg;
}

const svgCache = new Map<string, SVGSVGElement>();

/** Cached clone for DOM use (definition list, tabs). */
export function texElement(tex: string): SVGSVGElement {
  let svg = svgCache.get(tex);
  if (!svg) {
    svg = texToSvg(tex);
    svgCache.set(tex, svg);
  }
  return svg.cloneNode(true) as SVGSVGElement;
}

export interface TexImage {
  img: HTMLImageElement;
  /** Same glyphs stroked in the background color, drawn underneath for readability. */
  halo: HTMLImageElement;
  /** CSS pixels. */
  width: number;
  height: number;
  ready: boolean;
}

const imageCache = new Map<string, TexImage>();
let onImageReady: () => void = () => {};

/** Called when a cached label image finishes loading (the canvas redraws). */
export function setImageReadyHandler(fn: () => void): void {
  onImageReady = fn;
}

function exToPx(v: string | null, fontPx: number): number {
  const n = parseFloat(v ?? "0");
  return n * fontPx * EX_PER_EM;
}

/** Halo width in MathJax units (1000 = 1 em). */
const HALO_UNITS = 140;

function toImage(svg: SVGSVGElement, done: () => void): HTMLImageElement {
  const img = new Image();
  img.onload = done;
  img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(new XMLSerializer().serializeToString(svg));
  return img;
}

/**
 * Label image for the canvas, keyed by TeX, color, size and device pixel ratio.
 * Rasterized at device resolution: an SVG image drawn larger than its intrinsic
 * size is scaled as a bitmap and looks soft on HiDPI screens.
 */
export function texImage(tex: string, color: string, fontPx: number, halo: string, dpr: number): TexImage {
  const key = `${fontPx}|${color}|${halo}|${dpr}|${tex}`;
  const hit = imageCache.get(key);
  if (hit) return hit;
  const svg = texToSvg(tex);
  const width = exToPx(svg.getAttribute("width"), fontPx);
  const height = exToPx(svg.getAttribute("height"), fontPx);
  // Room for the halo around the glyphs.
  const pad = (HALO_UNITS / 1000) * fontPx;
  const vb = (svg.getAttribute("viewBox") ?? "0 0 0 0").split(/\s+/).map(Number);
  const padUnits = HALO_UNITS;
  svg.setAttribute("viewBox", `${vb[0]! - padUnits} ${vb[1]! - padUnits} ${vb[2]! + 2 * padUnits} ${vb[3]! + 2 * padUnits}`);
  const w = width + 2 * pad, h = height + 2 * pad;
  svg.setAttribute("width", `${w * dpr}`);
  svg.setAttribute("height", `${h * dpr}`);
  svg.removeAttribute("style");
  svg.setAttribute("color", color);
  const haloSvg = svg.cloneNode(true) as SVGSVGElement;
  haloSvg.setAttribute("color", halo);
  const g = haloSvg.querySelector("g");
  g?.setAttribute("stroke-width", String(HALO_UNITS));
  g?.setAttribute("stroke-linejoin", "round");
  let loaded = 0;
  const entry: TexImage = { img: null!, halo: null!, width: w, height: h, ready: false };
  const done = () => {
    if (++loaded === 2) {
      entry.ready = true;
      onImageReady();
    }
  };
  entry.img = toImage(svg, done);
  entry.halo = toImage(haloSvg, done);
  imageCache.set(key, entry);
  return entry;
}
