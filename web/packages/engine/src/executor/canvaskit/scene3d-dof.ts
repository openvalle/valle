import type {
  CanvasKit,
  GrDirectContext,
  Image,
  Shader,
  Surface,
} from "canvaskit-wasm";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";

export interface DeferredScene3dDof {
  width: number;
  height: number;
  x: number;
  y: number;
  inputWidth: number;
  inputHeight: number;
  uniforms: Float32Array;
  color: Uint8Array;
  depth: Uint8Array;
}

export function decodeDeferredScene3dDof(
  packet: Uint8Array,
): DeferredScene3dDof {
  if (packet.byteLength < 80) throw new Error("Truncated Scene3D DOF packet");
  const view = new DataView(
    packet.buffer,
    packet.byteOffset,
    packet.byteLength,
  );
  if (view.getUint32(0, true) !== 0x464f4456)
    throw new Error("Unsupported Scene3D DOF packet");
  const [width, height, x, y, inputWidth, inputHeight] = Array.from(
    { length: 6 },
    (_, i) => view.getUint32(4 + i * 4, true),
  );
  if (
    !width ||
    !height ||
    !inputWidth ||
    !inputHeight ||
    width! * height! > 16_777_216 ||
    x! + inputWidth! > width! ||
    y! + inputHeight! > height!
  )
    throw new Error("Invalid Scene3D DOF extent");
  const pixels = inputWidth! * inputHeight!;
  if (packet.byteLength !== 80 + pixels * 12)
    throw new Error("Invalid Scene3D DOF planes");
  const uniforms = Float32Array.from({ length: 13 }, (_, i) =>
    view.getFloat32(28 + i * 4, true),
  );
  if (
    !uniforms.every(Number.isFinite) ||
    uniforms[0] !== width ||
    uniforms[1] !== height ||
    uniforms[2]! <= 0 ||
    uniforms[3]! < 0 ||
    uniforms[3]! > 16 ||
    uniforms[12]! <= 0
  ) {
    throw new Error("Invalid Scene3D DOF uniforms");
  }
  return {
    width: width!,
    height: height!,
    x: x!,
    y: y!,
    inputWidth: inputWidth!,
    inputHeight: inputHeight!,
    uniforms,
    color: packet.subarray(80, 80 + pixels * 8),
    depth: packet.subarray(80 + pixels * 8),
  };
}

/** Context-owned scratch only. Returned COW snapshots belong to the ordinary resource cache. */
export class CanvasKitScene3dDof {
  private readonly builtins: CanvasKitBuiltinRuntime;
  private surface: Surface | null = null;
  private extent = "";
  constructor(
    private readonly ck: CanvasKit,
    private readonly context: GrDirectContext,
  ) {
    this.builtins = new CanvasKitBuiltinRuntime(ck);
    this.builtins.admit(["scene3dDof"]);
  }

  render(input: DeferredScene3dDof): Image {
    const ck = this.ck;
    const info = {
      width: input.inputWidth,
      height: input.inputHeight,
      colorType: ck.ColorType.RGBA_F16,
      alphaType: ck.AlphaType.Premul,
      colorSpace: ck.ColorSpace.SRGB,
    };
    const color = ck.MakeImage(info, input.color, input.inputWidth * 8);
    // Four byte planes in Alpha8 preserve every depth bit. No RGBA unpremultiplication,
    // color conversion or half-float depth quantization can affect occlusion decisions.
    const depth = ck.MakeImage(
      {
        width: input.inputWidth * 4,
        height: input.inputHeight,
        colorType: ck.ColorType.Alpha_8,
        alphaType: ck.AlphaType.Unpremul,
        colorSpace: ck.ColorSpace.SRGB,
      },
      input.depth,
      input.inputWidth * 4,
    );
    if (!color || !depth) {
      color?.delete();
      depth?.delete();
      throw new Error("Scene3D DOF input allocation failed");
    }
    let source: Shader | null = null,
      packed: Shader | null = null,
      shader: Shader | null = null;
    const paint = new ck.Paint();
    try {
      const extent = `${input.width}x${input.height}`;
      if (extent !== this.extent) {
        this.surface?.delete();
        this.surface = null;
        this.extent = "";
        this.surface = ck.MakeRenderTarget(this.context, {
          ...info,
          width: input.width,
          height: input.height,
        });
        if (!this.surface)
          throw new Error("Scene3D DOF output allocation failed");
        this.extent = extent;
      }
      source = color.makeShaderOptions(
        ck.TileMode.Decal,
        ck.TileMode.Decal,
        ck.FilterMode.Nearest,
        ck.MipmapMode.None,
        [1, 0, input.x, 0, 1, input.y, 0, 0, 1],
      );
      packed = depth.makeShaderOptions(
        ck.TileMode.Decal,
        ck.TileMode.Decal,
        ck.FilterMode.Nearest,
        ck.MipmapMode.None,
        [0.25, 0, input.x, 0, 1, input.y, 0, 0, 1],
      );
      shader = this.builtins.shader(
        "scene3dDof",
        input.uniforms,
        [source, packed],
        true,
      );
      paint.setBlendMode(ck.BlendMode.Src);
      paint.setShader(shader);
      this.surface!.getCanvas().drawPaint(paint);
      this.surface!.flush();
      return this.surface!.makeImageSnapshot();
    } finally {
      paint.delete();
      shader?.delete();
      source?.delete();
      packed?.delete();
      color.delete();
      depth.delete();
    }
  }

  dispose(): void {
    this.surface?.delete();
    this.surface = null;
    this.extent = "";
    this.builtins.dispose();
  }
}
