export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const COLS = 8;
const DOTS = Array.from({ length: COLS * COLS }, (_, i) => ({
  id: `c${i}`,
  x: 20.3 + (i % COLS) * (600 / COLS),
  y: 20.6 + Math.floor(i / COLS) * (320 / COLS),
  radius: 3.5 + (i % 3) * 0.25,
  color: i % 2 === 0 ? "#8b7bff" : "#22d3ee",
  delay: i / (COLS * COLS),
}));
export default function CircleGrid(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    {DOTS.map((dot) => (
      <Circle key={dot.id} cx={dot.x} cy={dot.y} r={dot.radius} fill={dot.color}
        style={{ opacity: clamp(ctx.seconds * 2 - dot.delay, 0, 1) }} />
    ))}
  </Scene>;
}
