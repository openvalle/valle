/** Fit the visible stage in device pixels, capped at 1080p without upscaling. */
export function previewRasterScale(
  canvas: { width: number; height: number },
  viewport: { width: number; height: number },
  pixelRatio = 1,
): number {
  const fit =
    viewport.width > 0 && viewport.height > 0
      ? Math.min(
          (viewport.width * pixelRatio) / canvas.width,
          (viewport.height * pixelRatio) / canvas.height,
        )
      : 1;
  return Math.min(1, 1920 / canvas.width, 1080 / canvas.height, fit);
}
