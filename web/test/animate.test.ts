import { afterEach, describe, expect, it, vi } from "vitest";
import type { Animate } from "../src/generated/Animate";
import { Animator } from "../src/ui/animate";

describe("animate", () => {
  afterEach(() => vi.unstubAllGlobals());

  // Drives the animator by hand: each queued frame runs at the next time in `times`.
  function run(integer: boolean, times: number[], mode: Partial<Animate> = {}): number[] {
    const frames: ((t: number) => void)[] = [];
    vi.stubGlobal("requestAnimationFrame", (f: (t: number) => void) => frames.push(f));
    vi.stubGlobal("performance", { now: () => 0 });
    const seen: number[] = [];
    const a = new Animator({ frame: (_k, v) => seen.push(v), done: () => {}, integer: () => integer });
    a.start([{ constant: "N", from: 5, to: 21, duration: 1, looped: false, repeat: false, ...mode }]);
    for (const t of times) frames.shift()?.(t);
    return seen;
  }

  it("integer constants take whole values only, without repeats", () => {
    const seen = run(true, [0, 100, 110, 300, 500, 700, 900, 1000]);
    expect(seen.every(Number.isInteger)).toBe(true);
    expect(seen.at(-1)).toBe(21);
    for (let i = 1; i < seen.length; i++) expect(seen[i]).not.toBe(seen[i - 1]);
  });

  it("real constants move continuously", () => {
    const seen = run(false, [300, 1000]);
    expect(Number.isInteger(seen[1])).toBe(false);
    expect(seen.at(-1)).toBe(21);
  });

  it("repeat runs at constant speed and jumps back to the start", () => {
    const seen = run(false, [250, 500, 1250, 2500], { from: 0, to: 1, repeat: true });
    expect(seen).toEqual([0, 0.25, 0.5, 0.25, 0.5]);
  });
});
