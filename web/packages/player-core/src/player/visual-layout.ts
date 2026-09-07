/** Packed ResourceRequest expected descriptor uses serde field names, not TS camelCase. */
export function canvaskitImportLayout(expected: {
  pixel_layout?: unknown;
  pixelLayout?: unknown;
}): string {
  return String(expected.pixel_layout ?? expected.pixelLayout ?? "");
}

export function assertCanvaskitImportLayout(expected: {
  pixel_layout?: unknown;
  pixelLayout?: unknown;
}): string {
  const layout = canvaskitImportLayout(expected);
  if (layout !== "rgba8" && layout !== "rgba16Float") {
    throw new Error(
      `browser visual resource expected ${layout || "missing pixel_layout"}, CanvasKit only imports rgba8/rgba16Float`,
    );
  }
  return layout;
}
