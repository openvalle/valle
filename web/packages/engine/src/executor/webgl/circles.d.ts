import type { CanvasKit, Canvas } from "canvaskit-wasm";
import type { CanvasKitGpuHost } from "./gpu-state.ts";
export class GpuCircles {
  constructor(
    host: CanvasKitGpuHost,
    options?: {
      f16?: boolean;
      roi?: boolean;
      nativeInstances?: boolean;
      pool?: boolean;
    },
  );
  readonly stats: {
    batches: number;
    circles: number;
    allocatedBytes: number;
    extractionMs: number;
    recordMs: number;
  };
  canCrop(draw: object, roots: readonly number[], matrix?: readonly number[]): boolean;
  beginFrame(): void;
  endFrame(): void;
  draw(
    ck: CanvasKit,
    canvas: Canvas,
    draw: object,
    roots: readonly unknown[],
    matrix: readonly number[],
  ): boolean;
  drawChildren(
    ck: CanvasKit,
    canvas: Canvas,
    draw: object,
    roots: readonly unknown[],
    matrix: readonly number[],
    fallback: (node: unknown) => void,
  ): void;
  batch(ck: CanvasKit, canvas: Canvas, draw: object, node: object): boolean;
  dispose(): void;
}
