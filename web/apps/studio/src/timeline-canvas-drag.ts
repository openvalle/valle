export interface CanvasDragOrigin {
  startX: number;
  startY: number;
  baseX: number;
  baseY: number;
}

export function crossedCanvasDragThreshold(
  origin: Pick<CanvasDragOrigin, "startX" | "startY">,
  clientX: number,
  clientY: number,
  thresholdPx: number,
): boolean {
  return Math.hypot(clientX - origin.startX, clientY - origin.startY) >= thresholdPx;
}

export function canvasDragPosition(
  origin: CanvasDragOrigin,
  clientX: number,
  clientY: number,
  displayWidth: number,
  displayHeight: number,
): { x: number; y: number } | null {
  if (displayWidth <= 0 || displayHeight <= 0) return null;
  return {
    x: origin.baseX + (clientX - origin.startX) / displayWidth,
    y: origin.baseY + (clientY - origin.startY) / displayHeight,
  };
}
