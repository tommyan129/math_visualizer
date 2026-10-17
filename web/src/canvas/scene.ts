// Reading the server's scene model: which targets show at a step, the clip in force,
// the axis names. Placement and step diffs are computed by the server; this only looks up.

import type { Clip } from "../generated/Clip";
import type { Scene } from "../generated/Scene";
import type { StepState } from "../generated/StepState";
import { identTex } from "../math/latex";

export function stepState(scene: Scene, step: number): StepState | undefined {
  return scene.steps[Math.min(step, scene.steps.length - 1)];
}

/** The user's override, else the step's cached state; members only. */
export function visibleAt(scene: Scene, step: number, key: string): boolean {
  if (!scene.members.includes(key)) return false;
  const t = scene.toggles[key];
  if (t !== undefined) return t;
  return stepState(scene, step)?.visible[key] ?? false;
}

/** Predict-then-reveal targets kept out of the toggle list at this step. */
export function concealedAt(scene: Scene, step: number): Set<string> {
  return new Set(stepState(scene, step)?.concealed ?? []);
}

/** None when the user turned the clip off; else the user's clip if adjusted, else Claude's. */
export function activeClip(scene: Scene | null): Clip | null {
  if (!scene || scene.clip_off) return null;
  return scene.user_clip ?? scene.clip;
}

/** Axis keys and their TeX (`2D(x, y)` without a scene). */
export function sceneAxes(scene: Scene | null): [[string, string], [string, string]] {
  const a = scene?.axes;
  if (!a || a.length < 2) return [["x", "y"], ["x", "y"]];
  return [[a[0]!.key, a[1]!.key], [identTex(a[0]!), identTex(a[1]!)]];
}

/** Tab order: default scenes first (creation order), then Claude scenes by number. */
export function sortedScenes(scenes: Scene[]): Scene[] {
  return [...scenes].sort((a, b) => (a.number === null ? 0 : 1) - (b.number === null ? 0 : 1) || (a.number ?? 0) - (b.number ?? 0) || a.order - b.order);
}
