import type { CanvasKit, GrDirectContext, Surface } from "canvaskit-wasm";
import dfgUrl from "../../../../../../crates/valle-motion/src/scene3d/raster/assets/dfg.rg16f" with { type: "file" };
import { GpuScene3D } from "./scene3d.ts";
import { resetCanvasKitContext } from "./gpu-state.ts";

export async function createScene3dGpu(
  CanvasKit: CanvasKit,
  canvas: HTMLCanvasElement,
  context: GrDirectContext,
  surface: Surface,
  isCurrent: () => boolean = () => true,
): Promise<GpuScene3D | null> {
  const gl = canvas.getContext("webgl2");
  if (
    !gl ||
    !gl.getExtension("EXT_color_buffer_float") ||
    !gl.getExtension("OES_texture_float_linear")
  )
    return null;
  const response = await fetch(new URL(dfgUrl, import.meta.url));
  if (!response.ok)
    throw new Error(`Scene3D BRDF resource failed: ${response.status}`);
  const bytes = await response.arrayBuffer();
  if (bytes.byteLength !== 1024)
    throw new Error("Scene3D BRDF resource has an invalid length");
  if (!isCurrent()) return null;
  return new GpuScene3D(
    {
      CanvasKit,
      gl,
      context,
      resetContext: () => resetCanvasKitContext(surface),
    },
    bytes,
  );
}
