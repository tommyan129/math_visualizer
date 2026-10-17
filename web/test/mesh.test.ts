// 3D geometry (stage 6a) against ASTs produced by the Rust resolver.

import { describe, expect, it } from "vitest";
import fixture from "./fixtures/state.json";
import type { State } from "../src/generated/State";
import { compile, compileRegion3, type Compiled, type Scalar, specOf } from "../src/worker/codegen";
import { clipLine3, patchOf, planeInBox, sampleParamSurface, sampleSpaceCurve, sampleSurface, spanShape3, type Box3 } from "../src/worker/mesh";
import { makeRuntime } from "../src/worker/numeric";

const state = fixture as unknown as State;
const fns = Object.values(state.functions).map((f) => f!);
const constKeys = Object.keys(state.constants);
const constIndex = new Map(constKeys.map((k, i) => [k, i]));
const fnIndex = new Map(fns.map((f, i) => [f.key, i]));
const R = makeRuntime();
const F: Scalar[] = [];
const compiled = new Map<string, Compiled>();
fns.forEach((f, i) => {
  const c = compile(specOf(f), { constIndex, fnIndex }, R, F, false);
  compiled.set(f.key, c);
  F[i] = c.table;
});
const C = Float64Array.from(constKeys.map((k) => state.constants[k]!.value));
const box: Box3 = { x0: -2, x1: 2, y0: -2, y1: 2, z0: -3, z1: 3 };

describe("surfaces", () => {
  it("evaluates g(x, y) and is callable with two arguments", () => {
    const g = compiled.get("sad")!.surface!;
    expect(g(2, 1, C)).toBe(3);
    expect(compiled.get("usesad")!.scalar(3, C)).toBe(8);
  });

  it("samples a grid with normals and full triangles", () => {
    const m = sampleSurface(compiled.get("sad")!.surface!, C, box, "coarse", null);
    const n = m.n + 1;
    expect(m.positions.length).toBe(n * n * 3);
    expect(m.indices.length).toBe(m.n * m.n * 6);
    expect(m.alpha.every((a) => a === 1)).toBe(true);
    // At the saddle point the normal is vertical.
    const c = (m.n / 2) * n + m.n / 2;
    expect(Math.abs(m.normals[c * 3 + 2]!)).toBeCloseTo(1, 6);
    expect(m.wire.length).toBeGreaterThan(0);
  });

  it("leaves out cells across a jump and at undefined points", () => {
    const m = sampleSurface(compiled.get("pole")!.surface!, C, box, "coarse", null);
    // x = 0 is a grid column (even cell count): undefined there, and 1/x jumps across it.
    expect(m.indices.length).toBeLessThan(m.n * m.n * 6);
    for (let i = 0; i < m.indices.length; i++) expect(Number.isFinite(m.positions[m.indices[i]! * 3 + 2]!)).toBe(true);
  });

  it("cuts by a clip region and masks a highlight range", () => {
    const sad = state.functions["sad"]!;
    const axes = [sad.inputs[0]!.key, sad.inputs[1]!.key, sad.output!.key];
    const clip = compileRegion3(state.highlights["H8"]!.range!, axes, { constIndex, fnIndex }, R, F);
    const m = sampleSurface(compiled.get("sad")!.surface!, C, box, "coarse", clip);
    const inside = m.alpha.reduce((s, a) => s + a, 0);
    expect(inside).toBeGreaterThan(0);
    expect(inside).toBeLessThan(m.alpha.length);
    const full = sampleSurface(compiled.get("sad")!.surface!, C, box, "coarse", null);
    const patch = patchOf(full, clip, C);
    expect(patch.positions.length).toBeGreaterThan(0);
    expect(patch.positions.length % 9).toBe(0);
    expect(patchOf(full, null, C).positions.length).toBe(full.indices.length * 3);
  });

  it("cuts a highlight patch along its boundary, not through edge midpoints", () => {
    // z = 0 plane over the box, region x² + y² < 1: every patch vertex lies inside the
    // closed disk, and the boundary vertices lie on the circle.
    const flat = sampleSurface(() => 0, C, box, "coarse", null);
    const disk = (x: number, y: number) => x * x + y * y < 1;
    const p = patchOf(flat, disk, C);
    let onCircle = 0, area = 0;
    for (let i = 0; i < p.positions.length; i += 3) {
      const r = Math.hypot(p.positions[i]! + p.origin[0], p.positions[i + 1]! + p.origin[1]);
      expect(r).toBeLessThan(1 + 1e-3);
      if (r > 1 - 1e-2) onCircle++;
    }
    for (let i = 0; i < p.positions.length; i += 9) {
      const [ax, ay, , bx, by, , cx, cy] = p.positions.slice(i, i + 9);
      area += Math.abs((bx! - ax!) * (cy! - ay!) - (cx! - ax!) * (by! - ay!)) / 2;
    }
    expect(onCircle).toBeGreaterThan(40);
    // The inscribed polygon on a 1/12-unit grid misses well under 1% of π.
    expect(area).toBeGreaterThan(Math.PI * 0.99);
    expect(area).toBeLessThan(Math.PI);
  });
});

describe("parametric 3D", () => {
  it("samples a sphere over both parameter ranges", () => {
    const c = compiled.get("sph")!;
    const m = sampleParamSurface(c.param2!, C, c.trange!(C), c.trange2!(C), box, "coarse", null);
    for (let i = 0; i < m.positions.length; i += 3) expect(Math.hypot(m.positions[i]!, m.positions[i + 1]!, m.positions[i + 2]!)).toBeCloseTo(1, 5);
    expect(c.trange!(C)[1]).toBeCloseTo(2 * Math.PI);
    expect(c.trange2!(C)[1]).toBeCloseTo(Math.PI);
  });

  it("samples a helix as one piece, broken by a clip", () => {
    const c = compiled.get("hx")!;
    const [t0, t1] = c.trange!(C);
    const l = sampleSpaceCurve(c.param!, C, t0!, t1!, box, "coarse", null);
    expect(l.offsets.length).toBe(2);
    const z = compileRegion3({ type: "Compare", id: 0, ty: { type: "Region" }, first: { type: "Name", id: 1, ty: { type: "Scalar" }, name: { key: "z", base: "z", sub: null, deriv: 0, inverse: false }, kind: "Input" }, rest: [{ op: "Lt", rhs: { type: "Num", id: 2, ty: { type: "Scalar" }, text: "2", int: true } }] } as never, ["x", "y", "z"], { constIndex, fnIndex }, R, F);
    const cut = sampleSpaceCurve(c.param!, C, t0!, t1!, box, "coarse", z);
    expect(cut.offsets.length).toBe(2);
    expect(cut.positions.length).toBeLessThan(l.positions.length);
  });
});

describe("3D spans", () => {
  it("classifies by rank", () => {
    expect(spanShape3([[0, 0, 0]]).dim).toBe(0);
    expect(spanShape3([[1, 2, 3], [2, 4, 6]]).dim).toBe(1);
    expect(spanShape3([[1, 0, 0], [0, 1, 1], [1, 1, 1]]).dim).toBe(2);
    expect(spanShape3([[1, 0, 0], [0, 1, 0], [0, 0, 1]]).dim).toBe(3);
    expect(spanShape3([[1, NaN, 0]]).dim).toBe(null);
    const n = spanShape3([[1, 0, 0], [0, 1, 0]]).dir!;
    expect(Math.abs(n[2])).toBeCloseTo(1);
  });

  it("cuts lines and planes to the box", () => {
    const seg = clipLine3([0, 0, 1], box)!;
    expect([seg[2], seg[5]]).toEqual([-3, 3]);
    expect(clipLine3([1, 0, 0], { ...box, y0: 1 })).toBe(null);
    // z = 0 plane: the box's square cross-section.
    const sq = planeInBox([0, 0, 1], box);
    expect(sq.length).toBe(4);
    for (const p of sq) expect(p[2]).toBeCloseTo(0);
    // A diagonal plane makes a hexagon in a cube.
    expect(planeInBox([1 / Math.sqrt(3), 1 / Math.sqrt(3), 1 / Math.sqrt(3)], { x0: -1, x1: 1, y0: -1, y1: 1, z0: -1, z1: 1 }).length).toBe(6);
  });
});
