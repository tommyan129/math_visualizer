// Dev page: our LaTeX output rendered by MathJax, next to the source TeX, for a
// side-by-side spacing check against claude.ai (the comparison itself is manual).

import "../src/style.css";
import fixture from "../test/fixtures/state.json";
import type { State } from "../src/generated/State";
import { colorOf } from "../src/canvas/plot";
import { definitionTex, type LatexCtx } from "../src/math/latex";
import { texElement } from "../src/math/mathjax";

const state = fixture as unknown as State;
const fns = Object.values(state.functions).map((f) => f!).sort((a, b) => a.order - b.order);
const ctx: LatexCtx = { colors: new Map(fns.map((f) => [f.key, colorOf(f)])) };
const plain: LatexCtx = { colors: new Map() };
const rows = document.getElementById("rows")!;

for (const f of fns) {
  const colored = definitionTex(f.name, f.inputs, f.body, ctx);
  const tex = definitionTex(f.name, f.inputs, f.body, plain);
  const tr = document.createElement("tr");
  const key = document.createElement("td");
  key.className = "key";
  key.textContent = f.key;
  const math = document.createElement("td");
  math.className = "math";
  math.append(texElement(colored));
  const code = document.createElement("td");
  const c = document.createElement("code");
  c.textContent = tex;
  code.append(c);
  tr.append(key, math, code);
  rows.append(tr);
}

document.getElementById("big")!.addEventListener("change", (e) => {
  const big = (e.target as HTMLInputElement).checked;
  document.querySelectorAll("td.math").forEach((td) => td.classList.toggle("big", big));
});
