import type { CanvasKit, RuntimeEffect, Shader, Image } from "canvaskit-wasm";
import { outputDitherPixels } from "./output-transform.ts";

import radialBlurSource from "../../../../../../crates/valle-draw/assets/shaders/radialblur.sksl" with { type: "text" };
import filmGrainSource from "../../../../../../crates/valle-draw/assets/shaders/filmgrain.sksl" with { type: "text" };
import lensDistortionSource from "../../../../../../crates/valle-draw/assets/shaders/lensdistortion.sksl" with { type: "text" };
import noiseDisplacementSource from "../../../../../../crates/valle-draw/assets/shaders/noisedisplacement.sksl" with { type: "text" };
import scene3dDofSource from "../../../../../../crates/valle-draw/assets/shaders/scene3ddof.sksl" with { type: "text" };
import scene3dDofCommon from "../../../../../../crates/valle-draw/assets/shaders/scene3ddofcommon.sksl" with { type: "text" };
import scene3dDofClassifySource from "../../../../../../crates/valle-draw/assets/shaders/scene3ddofclassify.sksl" with { type: "text" };
import scene3dDofDilate from "../../../../../../crates/valle-draw/assets/shaders/scene3ddofdilate.sksl" with { type: "text" };
import chromaticAberrationSource from "../../../../../../crates/valle-draw/assets/shaders/chromaticaberration.sksl" with { type: "text" };
import velocityBlurSource from "../../../../../../crates/valle-draw/assets/shaders/velocityblur.sksl" with { type: "text" };
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
import pyramidThreshold from "../../../../../../crates/valle-draw/assets/shaders/pyramidthreshold.sksl" with { type: "text" };
import pyramidDownsample from "../../../../../../crates/valle-draw/assets/shaders/pyramiddownsample.sksl" with { type: "text" };
import pyramidCoverageDownsample from "../../../../../../crates/valle-draw/assets/shaders/pyramidcoveragedownsample.sksl" with { type: "text" };
import pyramidBlur from "../../../../../../crates/valle-draw/assets/shaders/pyramidblur.sksl" with { type: "text" };
import pyramidUpsample from "../../../../../../crates/valle-draw/assets/shaders/pyramidupsample.sksl" with { type: "text" };
import pyramidUpsampleRows from "../../../../../../crates/valle-draw/assets/shaders/pyramidupsamplerows.sksl" with { type: "text" };
import pyramidUpsampleComposite from "../../../../../../crates/valle-draw/assets/shaders/pyramidupsamplecomposite.sksl" with { type: "text" };
import pyramidGlowUpsampleComposite from "../../../../../../crates/valle-draw/assets/shaders/pyramidglowupsamplecomposite.sksl" with { type: "text" };
import fadeSource from "../../../../../../crates/valle-draw/assets/shaders/fade.sksl" with { type: "text" };
import flyEyeSource from "../../../../../../crates/valle-draw/assets/shaders/flyeye.sksl" with { type: "text" };
import linearBlurSource from "../../../../../../crates/valle-draw/assets/shaders/linearblur.sksl" with { type: "text" };
import mosaicSource from "../../../../../../crates/valle-draw/assets/shaders/mosaic.sksl" with { type: "text" };
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

// CanvasKit exposes image shaders rather than raw image shaders. Transport depth bytes in
// Alpha8, where neither color management nor premultiplication can alter the numeric ABI.
const depthRead = "float4 b = floor(float4(packedDepth.eval(p + 0.5)) * 255.0 + 0.5);";
if (!scene3dDofCommon.includes(depthRead)) throw new Error("Scene3D depth shader ABI drifted");
const scene3dDepth = scene3dDofCommon
  .replace(
    depthRead,
    `float4 b = floor(float4(
    packedDepth.eval(p + float2(0.125, 0.5)).a,
    packedDepth.eval(p + float2(0.375, 0.5)).a,
    packedDepth.eval(p + float2(0.625, 0.5)).a,
    packedDepth.eval(p + float2(0.875, 0.5)).a) * 255.0 + 0.5);`,
  )
  .replace(
    "if (exponent == 254.0 && mantissa == 8388607.0)",
    "if ((exponent == 254.0 && mantissa == 8388607.0) || (exponent == 0.0 && mantissa == 0.0))",
  );

const BUILTIN_SOURCES = Object.freeze({
  chromaKey: chromaKeySource,
  filmGrain: filmGrainSource,
  lensDistortion: lensDistortionSource,
  noiseDisplacement: noiseDisplacementSource,
  scene3dDof: scene3dDepth + scene3dDofSource,
  scene3dDofClassify: scene3dDepth + scene3dDofClassifySource,
  scene3dDofDilate,
  radialBlur: radialBlurSource,
  chromaticAberration: chromaticAberrationSource,
  velocityBlur: velocityBlurSource,
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
  pyramidThreshold,
  pyramidDownsample,
  pyramidCoverageDownsample,
  pyramidBlur,
  pyramidUpsample,
  pyramidUpsampleRows,
  pyramidUpsampleComposite,
  pyramidGlowUpsampleComposite,
  flyEye: flyEyeSource,
  linearBlur: linearBlurSource,
  mosaic: mosaicSource,
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
export const BUILTIN_KERNELS = Object.freeze(Object.keys(BUILTIN_SOURCES) as BuiltinKernel[]);

/**
 * One backend-owned cache for the compositor's closed built-in kernel set. The SkSL source is
 * shared byte-for-byte with Native; compilation remains a backend admission step and happens
 * before a frame surface is touched.
 */
export class CanvasKitBuiltinRuntime {
  private noise: { seed: number; image: Image } | null = null;

  outputNoise(seed: number | null): Shader {
    const ck = this.CanvasKit;
    if (seed === null) return ck.Shader.MakeColor(ck.Color4f(0.5, 0.5, 0.5, 1), ck.ColorSpace.SRGB);
    if (this.noise?.seed !== seed) {
      const image = ck.MakeImage(
        {
          width: 128,
          height: 128,
          colorType: ck.ColorType.RGBA_8888,
          alphaType: ck.AlphaType.Opaque,
          colorSpace: ck.ColorSpace.SRGB,
        },
        outputDitherPixels(seed),
        128 * 4,
      );
      if (!image) throw new Error("cannot allocate output dither texture");
      this.noise?.image.delete();
      this.noise = { seed, image };
    }
    return this.noise.image.makeShaderOptions(
      ck.TileMode.Clamp,
      ck.TileMode.Clamp,
      ck.FilterMode.Nearest,
      ck.MipmapMode.None,
    );
  }
  private readonly effects = new Map<BuiltinKernel, RuntimeEffect>();

  constructor(private readonly CanvasKit: CanvasKit) {}

  admit(kernels: Iterable<BuiltinKernel>): void {
    for (const kernel of new Set(kernels)) {
      this.effect(kernel);
      if (
        TRANSITION_KERNELS.has(kernel) ||
        kernel.startsWith("pyramid") ||
        kernel === "lensDistortion" ||
        kernel === "scene3dDof"
      )
        this.effect("workingF16");
    }
  }

  shader(
    kernel: BuiltinKernel,
    uniforms: ArrayLike<number>,
    children: readonly Shader[],
    roundWorkingF16 = TRANSITION_KERNELS.has(kernel),
  ): Shader {
    const effect = this.effect(kernel);
    const packed = uniforms instanceof Float32Array ? uniforms : Float32Array.from(uniforms);
    const shader = effect.makeShaderWithChildren(packed, [...children]);
    if (!shader) throw new Error(`built-in CanvasKit kernel '${kernel}' rejected its typed ABI`);
    if (!roundWorkingF16) return shader;
    try {
      const rounded = this.effect("workingF16").makeShaderWithChildren(new Float32Array(), [
        shader,
      ]);
      if (!rounded) throw new Error("working F16 quantization rejected its ABI");
      return rounded;
    } finally {
      shader.delete();
    }
  }

  dispose(): void {
    this.noise?.image.delete();
    this.noise = null;
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
    if (!effect)
      throw new Error(`built-in CanvasKit kernel '${kernel}' failed admission: ${detail}`);
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
