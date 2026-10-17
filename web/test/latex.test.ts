// Mathematical notation for vectors and matrices (stage 4), on resolver output.

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/state.json";
import type { State } from "../src/generated/State";
import verifyFixture from "./fixtures/verify.json";
import { calcTex, definitionTex, DIFF_COLOR, exprTex, functionTex, identTex, isVectorName, latexCtx } from "../src/math/latex";

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

describe("split notation", () => {
  it("cases with one row per line", () => {
    expect(def("absx")).toBe("\\mathit{absx}\\left(x\\right) = \\begin{cases} -x, & x < 0 \\\\ x, & x \\ge 0 \\end{cases}");
  });

  it("a named subject is shown under the cases", () => {
    expect(def("sq2")).toContain("\\end{cases} \\\\ \\scriptstyle r \\,=\\, {x}^{2} \\end{gathered}");
  });
});

describe("parametric, polar and equation notation", () => {
  const fn = (k: string) => functionTex(state.functions[k]!, ctx);
  it("tuple output with the range under it", () => {
    expect(fn("pc")).toBe("\\begin{gathered} \\mathit{pc}\\left(t\\right) = \\left(\\cos\\left(t\\right), \\sin\\left(t\\right)\\right) \\\\ \\scriptstyle 0 \\,\\le\\, t \\,\\le\\, 2 \\pi \\end{gathered}");
  });
  it("polar without a range is a plain definition", () => {
    expect(fn("hc")).toBe("\\mathit{hc}\\left(\\theta\\right) = 1 + \\cos\\left(\\theta\\right)");
  });
  it("an equation is shown with its name", () => {
    expect(fn("Ci")).toBe("\\mathit{Ci}\\colon\\; {x}^{2} + {y}^{2} = 1");
  });
});

describe("family notation", () => {
  it("Ш with the index range and the body in braces", () => {
    expect(functionTex(state.functions["Oc"]!, ctx)).toBe("\\mathit{Oc}\\colon\\; \\Sha_{j=1}^{3} \\left\\{ {x}^{2} + {y}^{2} = {j}^{2} \\right\\}");
    expect(functionTex(state.functions["Dv"]!, ctx)).toContain("\\mathit{Dv}\\left(t\\right) = \\Sha_{j=0}^{n} \\left\\{ \\left(j, t\\right) \\right\\}");
    // A family of arrows is a set, so its name has no vector arrow.
    expect(isVectorName(state.functions["Af"]!)).toBe(false);
    expect(functionTex(state.functions["Af"]!, ctx)).toMatch(/^\\mathit\{Af\} = \\Sha_\{j=0\}\^\{n\} \\left\\\{ \\operatorname\{arrow\}/);
  });
});

describe("calculation steps (stage 5c)", () => {
  const vs = verifyFixture as unknown as State;
  const c = vs.functions["C"]!;
  const n = parseInt(DIFF_COLOR.slice(1), 16);
  const colored = `\\textcolor[RGB]{${(n >> 16) & 255},${(n >> 8) & 255},${n & 255}}`;
  const lines = (count: number) => calcTex(c, vs.functions, ctx, count).split(" \\\\ ");

  it("aligned lines, the first with the head", () => {
    const t = calcTex(c, vs.functions, ctx, 3);
    expect(t.startsWith("C\\colon\\; \\begin{aligned} f\\left(x\\right) &= ")).toBe(true);
    expect(lines(3).length).toBe(3);
    expect(t).toContain("&= ");
  });

  it("colors only what changed from the previous line", () => {
    // Both squares were expanded: two colored groups, nothing in the first line.
    expect(lines(2)[1]!.split(colored).length - 1).toBe(2);
    expect(lines(2)[0]!).not.toContain(colored);
  });

  it("shows only the steps revealed so far", () => {
    expect(lines(1).length).toBe(1);
  });

  it("integral relation on the first arrow", () => {
    expect(calcTex(vs.functions["Ci"]!, vs.functions, ctx, 2)).toContain("\\xrightarrow{\\int}");
  });
});
