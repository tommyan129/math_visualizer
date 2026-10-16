// Ш, the operator of repeated definitions `family(...; 0 <= k <= n)` (stage-5a decision).
// The TeX font has no Ш, so the glyph is drawn in its style: the ∏ large-operator outline
// flipped vertically (bar at the bottom, bracketed serifs on top) with a third stem,
// serifs narrowed (variant B). Units: 1/1000 em, y up; MathJax adds the leading M and Z.
// `\Sha` is defined like `\prod` (a large operator whose limits move to the side in
// text style).

import { Configuration } from "@mathjax/src/js/input/tex/Configuration.js";
import { CharacterMap } from "@mathjax/src/js/input/tex/TokenMap.js";
import ParseMethods from "@mathjax/src/js/input/tex/ParseMethods.js";

const SHA = 0x428;

/** [height, depth, width, path] for display (`-largeop`) and text size (`-smallop`, `normal`). */
const LARGE: [number, number, number, string] = [0.95, 0.45, 1.432, "56 -450L1376 -450L1376 -396L1369 -396C1309 -396 1268 -374 1256 -312L1256 811C1266 874 1309 896 1368 896L1376 896L1376 950L976 950L976 896L983 896C1041 896 1080 876 1094 820L1096 811L1096 -396L796 -396L796 811C806 874 849 896 908 896L916 896L916 950L516 950L516 896L523 896C581 896 620 876 634 820L636 811L636 -396L336 -396L336 811C346 874 389 896 448 896L456 896L456 950L56 950L56 896L63 896C121 896 160 876 174 820L176 811L176 -312C167 -346 158 -366 131 -381C115 -390 90 -396 64 -396L56 -396"];
const SMALL: [number, number, number, string] = [0.75, 0.25, 0.988, "55 -250L933 -250L933 -210L928 -210C888 -210 861 -194 853 -148L853 647C860 694 888 710 928 710L933 710L933 750L667 750L667 710L672 710C710 710 736 695 746 654L747 647L747 -210L547 -210L547 647C554 694 582 710 622 710L627 710L627 750L361 750L361 710L366 710C404 710 430 695 440 654L441 647L441 -210L241 -210L241 647C248 694 276 710 316 710L321 710L321 750L55 750L55 710L60 710C98 710 124 695 134 654L135 647L135 -148C129 -173 123 -188 105 -199C94 -206 77 -210 60 -210L55 -210"];

// Same parse method as `\\prod` (MathJax types it for its own maps only).
const mathchar0mo = ParseMethods.mathchar0mo as unknown as ConstructorParameters<typeof CharacterMap>[1];
new CharacterMap("mv-sha", mathchar0mo, { Sha: ["\u0428", { movesupsub: true, largeop: true, movablelimits: true, symmetric: true }] });
/** TeX package name providing `\Sha`. */
export const SHA_PACKAGE = "mvsha";
Configuration.create(SHA_PACKAGE, { handler: { macro: ["mv-sha"] } });

/** Adds the Ш glyph to an SVG output's font. */
export function defineSha(font: { defineChars(name: string, chars: Record<number, unknown>): void }): void {
  const ch = ([h, d, w, p]: [number, number, number, string]) => ({ [SHA]: [h, d, w, { p }] });
  font.defineChars("-largeop", ch(LARGE));
  font.defineChars("-smallop", ch(SMALL));
  font.defineChars("normal", ch(SMALL));
}
