// `animate` playback: a constant runs from its start to its end value over the
// duration on step entry (forward only). Without `loop` / `repeat` it stops at the end
// value; with `loop` it goes back and forth, with `repeat` it runs start to end at
// constant speed over and over, until stopped (slider grabbed, step left).
// Values are local, like a slider drag; the server already holds the end value.

import type { Animate } from "../generated/Animate";

export interface AnimatorHooks {
  frame(constant: string, value: number): void;
  /** Playback of `constant` ended (finished or stopped). */
  done(constant: string, value: number): void;
  /** Integer constants step through whole numbers (a `family` / `sum` bound must stay integral). */
  integer(constant: string): boolean;
}

interface Run {
  a: Animate;
  t0: number;
  value: number;
}

const ease = (t: number) => (t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2);

export class Animator {
  private runs = new Map<string, Run>();
  private raf = 0;

  constructor(private hooks: AnimatorHooks) {}

  playing(constant: string): boolean {
    return this.runs.has(constant);
  }

  start(list: Animate[]): void {
    const t0 = performance.now();
    for (const a of list) {
      this.stop(a.constant);
      this.runs.set(a.constant, { a, t0, value: a.from });
      this.hooks.frame(a.constant, a.from);
    }
    if (!this.raf && this.runs.size > 0) this.raf = requestAnimationFrame((t) => this.tick(t));
  }

  stop(constant: string): void {
    const r = this.runs.get(constant);
    if (!r) return;
    this.runs.delete(constant);
    this.hooks.done(constant, r.value);
  }

  stopAll(): void {
    for (const k of [...this.runs.keys()]) this.stop(k);
  }

  private tick(now: number): void {
    this.raf = 0;
    for (const [k, r] of [...this.runs]) {
      const d = Math.max(0.05, r.a.duration) * 1000;
      const t = (now - r.t0) / d;
      if (!r.a.looped && !r.a.repeat && t >= 1) {
        if (r.value !== r.a.to) {
          r.value = r.a.to;
          this.hooks.frame(k, r.value);
        }
        this.runs.delete(k);
        this.hooks.done(k, r.value);
        continue;
      }
      // Back and forth (0 → 1 → 0 …, eased), or start to end at constant speed over and over.
      const phase = r.a.looped ? ease(1 - Math.abs(1 - (t % 2))) : r.a.repeat ? t % 1 : ease(t);
      const v = r.a.from + (r.a.to - r.a.from) * Math.min(1, Math.max(0, phase));
      const value = this.hooks.integer(k) ? Math.round(v) : v;
      if (value === r.value) continue;
      r.value = value;
      this.hooks.frame(k, r.value);
    }
    if (this.runs.size > 0) this.raf = requestAnimationFrame((t) => this.tick(t));
  }
}
