import type { Expr } from "../../src/generated/Expr";

/** `name_[i, j]` with integer literals, as the resolver builds it. */
export function parseLike(name: string, idx: number[]): Expr {
  const ident = { key: name, base: name, accent: null, sub: null, sup: null, deriv: 0, inverse: false };
  const base: Expr = { type: "Name", id: 1, ty: { type: "Matrix", m: 2, n: 2 }, name: ident, kind: "Value" };
  const indices: Expr[] = idx.map((i, k) => ({ type: "Num", id: 2 + k, ty: { type: "Scalar" }, text: String(i), int: true }));
  return { type: "Index", id: 0, ty: { type: "Scalar" }, base, indices };
}
