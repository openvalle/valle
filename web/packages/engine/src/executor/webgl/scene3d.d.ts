import type { CanvasKit, GrDirectContext, Image } from "canvaskit-wasm";
export interface Scene3dGpuHost {
  CanvasKit: CanvasKit;
  gl: WebGL2RenderingContext;
  context: GrDirectContext;
  resetContext(): void;
}
export interface Scene3dGpuRequest {
  width: number;
  height: number;
  scene: Record<string, any>;
  frame: Record<string, any>;
  lightingInput?: Float32Array;
}
export class GpuScene3D {
  constructor(host: Scene3dGpuHost, dfgBytes: ArrayBuffer);
  readonly stats: {
    renders: number;
    prepareMs: number;
    recordMs: number;
    allocatedBytes: number;
    fallbacks: number;
    visibilityAllocations: number;
  };
  accepts(request: Scene3dGpuRequest): boolean;
  render(request: Scene3dGpuRequest): Promise<Image>;
  dispose(): void;
}
