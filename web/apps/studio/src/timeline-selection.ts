export interface TimelineHitRect {
  clipId?: string;
  rect: { x: number; y: number; w: number; h: number };
}

export interface DisplayRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export function topHitForClip<T extends TimelineHitRect>(
  hits: ReadonlyArray<T>,
  clipId: string,
): T | null {
  for (let index = hits.length - 1; index >= 0; index -= 1) {
    if (hits[index].clipId === clipId) return hits[index];
  }
  return null;
}

export function hitAtDisplayPoint<T extends TimelineHitRect>(
  hits: ReadonlyArray<T>,
  clientX: number,
  clientY: number,
  display: DisplayRect,
  canvasWidth: number,
  canvasHeight: number,
): T | null {
  if (display.width <= 0 || display.height <= 0 || canvasWidth <= 0 || canvasHeight <= 0) return null;
  const x = ((clientX - display.left) / display.width) * canvasWidth;
  const y = ((clientY - display.top) / display.height) * canvasHeight;
  for (let index = hits.length - 1; index >= 0; index -= 1) {
    const hit = hits[index];
    const rect = hit.rect;
    if (x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h) return hit;
  }
  return null;
}
