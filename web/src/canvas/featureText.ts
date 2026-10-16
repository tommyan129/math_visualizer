// Words and coordinate text for feature points (panel, hover, labels).

import type { FeatureClass } from "../generated/FeatureClass";
import type { FeaturePt } from "../worker/features";

export const FEATURE_WORD: Record<FeatureClass, string> = {
  Root: "근",
  TangentRoot: "접하는 근",
  Intersection: "교점",
  TangentIntersection: "접하는 교점",
  YIntercept: "y절편",
  Max: "극대",
  Min: "극소",
  Inflection: "변곡점",
  IntervalMax: "최댓값",
  IntervalMin: "최솟값",
  Hole: "구멍",
  Filled: "구멍 자리의 값",
  Asymptote: "수직 점근선",
  Jump: "점프",
  Endpoint: "정의역 끝점",
  Cusp: "뾰족점",
  HorizontalTangent: "수평 접선",
  VerticalTangent: "수직 접선",
};

/** Up to `digits` decimals, trailing zeros dropped, real minus sign. */
export function fmtCoord(v: number, digits = 3): string {
  const s = Number(v.toFixed(digits));
  return String(Object.is(s, -0) ? 0 : s).replace("-", "−");
}

/** `(1, −2)` in the scene's axes; a vertical asymptote as `x = 1`. */
export function pointText(p: FeaturePt, axis: string, digits = 3): string {
  if (p.cls === "Asymptote" || !Number.isFinite(p.vy)) return `${axis} = ${fmtCoord(p.vx, digits)}`;
  return `(${fmtCoord(p.vx, digits)}, ${fmtCoord(p.vy, digits)})`;
}
