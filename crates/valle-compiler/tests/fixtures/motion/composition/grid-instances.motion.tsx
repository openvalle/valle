export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const COLS = 8;
const CELLS = Array.from({ length: COLS * COLS }, (_, i) => {
  const c = i % COLS;
  const r = Math.floor(i / COLS);
  const dx = c - COLS / 2;
  const dy = r - COLS / 2;
  return { id: `t${i}`, x: 8 + c * (624 / COLS), y: 6 + r * (348 / COLS), d: Math.sqrt(dx * dx + dy * dy) / COLS };
});
export default function Grid(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      {CELLS.map((cell) => (
        <View key={cell.id} className="absolute" style={{ left: cell.x, top: cell.y, width: 4, height: 4,
          backgroundColor: "#8b7bff", opacity: clamp(ctx.seconds * 2 - cell.d, 0, 1),
          transform: `rotate(${ctx.seconds * 90 + cell.d * 180}deg)` }} />
      ))}
    </Scene>
  );
}
