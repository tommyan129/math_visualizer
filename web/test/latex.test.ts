// Mathematical notation for vectors and matrices (stage 4), on resolver output.

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/state.json";
import type { State } from "../src/generated/State";
import { definitionTex, exprTex, identTex, latexCtx } from "../src/math/latex";

const state = fixture as unknown as State;
const ctx = { colors: new Map<string, string>() };
const def = (k: string) => {
  const f = state.functions[k]!;
  return definitionTex(f.name, f.inputs, f.body, ctx);
};

describe("matrix notation", () => {
  it("literals as pmatrix, values without parentheses", () => {
    expect(def("M2")).toBe("M2 = \\begin{pmatrix}1 & 2 \\\\ 3 & 4\\end{pmatrix}".replace("M2", "\\mathit{M2}"));
    expect(def("vv")).toBe("\\mathit{vv} = \\begin{pmatrix}1 \\\\ a\\end{pmatrix}");
  });

  it("matmul as juxtaposition, element as a lowercase subscript", () => {
    expect(exprTex(state.functions["rotx"]!.body, ctx)).toBe("\\left(\\mathit{Rot}\\left(b\\right)\\begin{pmatrix}x \\\\ 1\\end{pmatrix}\\right)_{2}");
  });

  it("transpose, inverse, power, det, tr", () => {
    const t = exprTex(state.functions["linalg"]!.body, ctx);
    expect(t).toContain("\\det\\left(\\mathit{M2}\\right)");
    expect(t).toContain("\\operatorname{tr}\\left({\\mathit{M2}}^{-1}\\right)");
    expect(t).toContain("\\left({\\mathit{M2}}^{2}\\right)_{12}");
    expect(t).toContain("\\left({\\mathit{M2}}^{\\mathsf{T}}\\right)_{12}");
  });

  it("span and arrow", () => {
    expect(def("Sp")).toBe("\\mathit{Sp} = \\operatorname{span}\\left(\\mathit{vv}, \\begin{pmatrix}0 \\\\ 1\\end{pmatrix}\\right)");
    expect(def("Ar")).toContain("\\operatorname{arrow}\\left(\\mathit{vv};\\, ");
  });
});

describe("vector arrows", () => {
  const vctx = latexCtx(Object.values(state.functions).map((f) => f!), () => "");
  const vdef = (k: string) => {
    const f = state.functions[k]!;
    return definitionTex(f.name, f.inputs, f.body, { ...vctx, colors: new Map() });
  };

  it("named vectors and arrows get an arrow; matrices and spans do not", () => {
    expect(vdef("vv")).toMatch(/^\\overrightarrow\{\\mathit\{vv\}\} = /);
    expect(vdef("Sp")).toBe("\\mathit{Sp} = \\operatorname{span}\\left(\\overrightarrow{\\mathit{vv}}, \\begin{pmatrix}0 \\\\ 1\\end{pmatrix}\\right)");
    expect(vdef("Ar")).toMatch(/^\\overrightarrow\{\\mathit\{Ar\}\} = /);
    expect(vdef("M2")).toMatch(/^\\mathit\{M2\} = /);
  });

  it("\\vec over a single letter or a Greek name, accents kept", () => {
    const id = (base: string, accent: string | null = null) => ({ key: base, base, accent, sub: null, sup: null, deriv: 0, inverse: false });
    expect(identTex(id("v"), undefined, true)).toBe("\\vec{v}");
    expect(identTex(id("\\alpha"), undefined, true)).toBe("\\vec{\\alpha}");
    expect(identTex(id("v", "hat"), undefined, true)).toBe("\\hat{v}");
  });
});

describe("element subscripts", () => {
  it("A_[1, 2] → a₁₂", async () => {
    const { parseLike } = await import("./util/expr");
    expect(exprTex(parseLike("A", [1, 2]), ctx)).toBe("a_{12}");
    expect(exprTex(parseLike("v", [2]), ctx)).toBe("v_{2}");
  });
});
