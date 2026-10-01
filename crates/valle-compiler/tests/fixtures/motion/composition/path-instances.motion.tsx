export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const STAR = path("M 0 -8 L 2 -2 L 8 0 L 2 2 L 0 8 L -2 2 L -8 0 L -2 -2 Z");
const COLS = 8;
const DOTS = Array.from({ length: COLS * COLS }, (_, i) => ({
  id: `p${i}`,
  x: 20.3 + (i % COLS) * (600 / COLS),
  y: 20.6 + Math.floor(i / COLS) * (320 / COLS),
  color: i % 2 === 0 ? "#8b7bff80" : "#22d3ee",
  delay: i / (COLS * COLS),
}));
export default function PathGrid(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    {DOTS.map((dot) => (
      <Path key={dot.id} d={STAR} fill={dot.color}
        style={{ translate: point(dot.x + ctx.seconds * 3, dot.y) }} />
    ))}
  </Scene>;
}
