import type { CanvasKit, RuntimeEffect, Shader } from "canvaskit-wasm";

import radialBlurSource from "../../../../../../crates/valle-draw/assets/shaders/radialblur.sksl" with { type: "text" };
import filmGrainSource from "../../../../../../crates/valle-draw/assets/shaders/filmgrain.sksl" with { type: "text" };
import chromaticAberrationSource from "../../../../../../crates/valle-draw/assets/shaders/chromaticaberration.sksl" with { type: "text" };
import chromaKeySource from "../../../../../../crates/valle-draw/assets/shaders/chromakey.sksl" with { type: "text" };
import circleOpenSource from "../../../../../../crates/valle-draw/assets/shaders/circleopen.sksl" with { type: "text" };
import colorGradeSource from "../../../../../../crates/valle-draw/assets/shaders/colorgrade.sksl" with { type: "text" };
import creativeBlendSource from "../../../../../../crates/valle-draw/assets/shaders/creativeblend.sksl" with { type: "text" };
import maskCoverageSource from "../../../../../../crates/valle-draw/assets/shaders/maskcoverage.sksl" with { type: "text" };
import crossWarpSource from "../../../../../../crates/valle-draw/assets/shaders/crosswarp.sksl" with { type: "text" };
import directionalBlurSource from "../../../../../../crates/valle-draw/assets/shaders/directionalblur.sksl" with { type: "text" };
import directionalWarpSource from "../../../../../../crates/valle-draw/assets/shaders/directionalwarp.sksl" with { type: "text" };
import dreamyZoomSource from "../../../../../../crates/valle-draw/assets/shaders/dreamyzoom.sksl" with { type: "text" };
import transitionInputSource from "../../../../../../crates/valle-draw/assets/shaders/transitioninput.sksl" with { type: "text" };
import workingF16Source from "../../../../../../crates/valle-draw/assets/shaders/workingf16.sksl" with { type: "text" };
import fadeSource from "../../../../../../crates/valle-draw/assets/shaders/fade.sksl" with { type: "text" };
import flyEyeSource from "../../../../../../crates/valle-draw/assets/shaders/flyeye.sksl" with { type: "text" };
import linearBlurSource from "../../../../../../crates/valle-draw/assets/shaders/linearblur.sksl" with { type: "text" };
import mosaicSource from "../../../../../../crates/valle-draw/assets/shaders/mosaic.sksl" with { type: "text" };
import motionGlassSource from "../../../../../../crates/valle-draw/assets/shaders/motionglass.sksl" with { type: "text" };
import multiplyBlendSource from "../../../../../../crates/valle-draw/assets/shaders/multiplyblend.sksl" with { type: "text" };
import normalizeInputSource from "../../../../../../crates/valle-draw/assets/shaders/normalizeinput.sksl" with { type: "text" };
import opacitySource from "../../../../../../crates/valle-draw/assets/shaders/opacity.sksl" with { type: "text" };
import outputTransformSource from "../../../../../../crates/valle-draw/assets/shaders/outputtransform.sksl" with { type: "text" };
import perlinSource from "../../../../../../crates/valle-draw/assets/shaders/perlin.sksl" with { type: "text" };
import rippleSource from "../../../../../../crates/valle-draw/assets/shaders/ripple.sksl" with { type: "text" };
import simpleZoomSource from "../../../../../../crates/valle-draw/assets/shaders/simplezoom.sksl" with { type: "text" };
import spotlightSource from "../../../../../../crates/valle-draw/assets/shaders/spotlight.sksl" with { type: "text" };
import wipeLeftSource from "../../../../../../crates/valle-draw/assets/shaders/wipeleft.sksl" with { type: "text" };
import wipeRightSource from "../../../../../../crates/valle-draw/assets/shaders/wiperight.sksl" with { type: "text" };

export type BuiltinKernel = keyof typeof BUILTIN_SOURCES;
export const MOTION_GLASS_UNIFORM_FLOATS = 740;

const BUILTIN_SOURCES = Object.freeze({
  chromaKey: chromaKeySource,
  filmGrain: filmGrainSource,
  radialBlur: radialBlurSource,
  chromaticAberration: chromaticAberrationSource,
  circleOpen: circleOpenSource,
  colorGrade: colorGradeSource,
  creativeBlend: creativeBlendSource,
  maskCoverage: maskCoverageSource,
  crossWarp: crossWarpSource,
  directionalBlur: directionalBlurSource,
  directionalWarp: directionalWarpSource,
  dreamyZoom: dreamyZoomSource,
  fade: fadeSource,
  transitionInput: transitionInputSource,
  workingF16: workingF16Source,
  flyEye: flyEyeSource,
  linearBlur: linearBlurSource,
  mosaic: mosaicSource,
  motionGlass: motionGlassSource,
  multiplyBlend: multiplyBlendSource,
  normalizeInput: normalizeInputSource,
  opacity: opacitySource,
  outputTransform: outputTransformSource,
  perlin: perlinSource,
  ripple: rippleSource,
  simpleZoom: simpleZoomSource,
  spotlight: spotlightSource,
  wipeLeft: wipeLeftSource,
  wipeRight: wipeRightSource,
});

/** Closed built-in set, exported so admission tests cannot silently omit a newly added kernel. */
export const BUILTIN_KERNELS = Object.freeze(
  Object.keys(BUILTIN_SOURCES) as BuiltinKernel[],
);

/**
 * One backend-owned cache for the compositor's closed built-in kernel set. The SkSL source is
 * shared byte-for-byte with Native; compilation remains a backend admission step and happens
 * before a frame surface is touched.
 */
export class CanvasKitBuiltinRuntime {
  private readonly effects = new Map<BuiltinKernel, RuntimeEffect>();

  constructor(private readonly CanvasKit: CanvasKit) {}

  admit(kernels: Iterable<BuiltinKernel>): void {
    for (const kernel of new Set(kernels)) {
      this.effect(kernel);
      if (TRANSITION_KERNELS.has(kernel)) this.effect("workingF16");
    }
  }

  shader(kernel: BuiltinKernel, uniforms: ArrayLike<number>, children: readonly Shader[]): Shader {
    const effect = this.effect(kernel);
    const packed = uniforms instanceof Float32Array ? uniforms : Float32Array.from(uniforms);
    const shader = effect.makeShaderWithChildren(packed, [...children]);
    if (!shader) throw new Error(`built-in CanvasKit kernel '${kernel}' rejected its typed ABI`);
    if (!TRANSITION_KERNELS.has(kernel)) return shader;
    try {
      const rounded = this.effect("workingF16").makeShaderWithChildren(new Float32Array(), [shader]);
      if (!rounded) throw new Error("working F16 quantization rejected its ABI");
      return rounded;
    } finally { shader.delete(); }
  }

  dispose(): void {
    for (const effect of this.effects.values()) effect.delete();
    this.effects.clear();
  }

  private effect(kernel: BuiltinKernel): RuntimeEffect {
    const cached = this.effects.get(kernel);
    if (cached) return cached;
    let detail = "";
    const effect = this.CanvasKit.RuntimeEffect.Make(BUILTIN_SOURCES[kernel], (message) => {
      detail = message;
    });
    if (!effect) throw new Error(`built-in CanvasKit kernel '${kernel}' failed admission: ${detail}`);
    if (kernel === "motionGlass"
      && (effect.getUniformFloatCount() !== MOTION_GLASS_UNIFORM_FLOATS
        || effect.getUniformCount() !== 20
        || effect.getUniformName(0) !== "header0"
        || effect.getUniformName(19) !== "pathPointPairs")) {
      effect.delete();
      throw new Error("built-in CanvasKit Motion Glass uniform ABI drifted");
    }
    this.effects.set(kernel, effect);
    return effect;
  }
}

export const TRANSITION_KERNELS = new Set<BuiltinKernel>([
  "fade",
  "wipeLeft",
  "wipeRight",
  "circleOpen",
  "simpleZoom",
  "crossWarp",
  "linearBlur",
  "directionalWarp",
  "dreamyZoom",
  "ripple",
  "flyEye",
  "multiplyBlend",
  "perlin",
]);
