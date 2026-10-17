// The canvas card: the 2D plot (Canvas 2D) or, for a `3D(x, y, z)` scene, the 3D plot
// (three.js). The 2D plot always gets the state: numeric verification runs there.

import type { CheckResult } from "../generated/CheckResult";
import type { FeaturePoints } from "../generated/FeaturePoints";
import type { Nav } from "../generated/Nav";
import type { SpanDim } from "../generated/SpanDim";
import type { State } from "../generated/State";
import type { ViewBox } from "../generated/ViewBox";
import type { Warning } from "../generated/Warning";
import type { CurveResult } from "../worker/worker";
import { Plot, type PlotCallbacks } from "./plot";
import { Plot3D } from "./plot3d";

export class Plots {
  private p2: Plot;
  private p3: Plot3D;
  private is3d = false;

  constructor(host: HTMLElement, cb: PlotCallbacks) {
    // While a 3D scene is shown, the 2D plot reports only panel refreshes (checks arriving).
    this.p2 = new Plot(host, { rendered: (rev, w) => (!this.is3d ? cb.rendered(rev, w) : rev < 0 && cb.rendered(-1, this.warnings())) });
    this.p3 = new Plot3D(host, { rendered: (rev, w) => this.is3d && cb.rendered(rev, w) });
  }

  setState(state: State, revision: number, nav: Nav, workspace = ""): void {
    const scene = (nav.scene && state.scenes[nav.scene]) || null;
    this.is3d = scene?.axes.length === 3;
    this.p2.setActive(!this.is3d);
    this.p3.setActive(this.is3d);
    if (this.is3d) {
      this.p2.setState(state, revision, { ...nav, scene: null });
      this.p3.setState(state, revision, nav, workspace);
    } else {
      this.p2.setState(state, revision, nav);
      this.p3.setState(state, revision, { ...nav, scene: null });
    }
  }

  setConstant(key: string, value: number, final: boolean): void {
    this.p2.setConstant(key, value, final);
    this.p3.setConstant(key, value, final);
  }

  clearOverride(key: string): void {
    this.p2.clearOverride(key);
    this.p3.clearOverride(key);
  }

  releaseConstant(key: string): void {
    this.p2.releaseConstant(key);
    this.p3.releaseConstant(key);
  }

  orderViolation(key: string, value: number): string | null {
    return this.p2.orderViolation(key, value);
  }

  warnings(): Warning[] {
    return this.is3d ? this.p3.warnings() : this.p2.warnings();
  }

  warningsFor(key: string): Warning[] {
    return this.is3d ? this.p3.warningsFor(key) : this.p2.warningsFor(key);
  }

  sceneWarnings(): Warning[] {
    return this.is3d ? this.p3.sceneWarnings() : this.p2.sceneWarnings();
  }

  spanDims(): SpanDim[] {
    return this.is3d ? this.p3.spanDims() : this.p2.spanDims();
  }

  featurePoints(): FeaturePoints[] {
    return this.is3d ? [] : this.p2.featurePoints();
  }

  featureResult(key: string): CurveResult | undefined {
    return this.p2.featureResult(key);
  }

  checks(): CheckResult[] {
    return this.p2.checks();
  }

  coordLabels(key: string): boolean {
    return this.p2.coordLabels(key);
  }

  toggleCoordLabels(key: string): void {
    this.p2.toggleCoordLabels(key);
  }

  box(): ViewBox {
    return this.is3d ? this.p3.box() : this.p2.box();
  }

  png(maxSide: number): { data: string; width: number; height: number } {
    return this.is3d ? this.p3.png(maxSide) : this.p2.png(maxSide);
  }

  zoom(factor: number): void {
    if (this.is3d) this.p3.zoom(factor);
    else this.p2.zoom(factor);
  }

  resetView(): void {
    if (this.is3d) this.p3.resetView();
    else this.p2.resetView();
  }
}
