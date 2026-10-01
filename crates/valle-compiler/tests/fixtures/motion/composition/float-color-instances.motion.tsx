export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

const CELLS = Array.from({ length: 64 }, (_, i) => ({
  id: `c${i}`, x: (i % 8) * 80, y: Math.floor(i / 8) * 45
}));

export default function FloatInstances(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      {CELLS.map((cell) => (
        <View key={cell.id} className="absolute" style={{
          left: cell.x, top: cell.y, width: 80, height: 45,
          backgroundColor: interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
            { colorSpace: "oklch" })
        }} />
      ))}
    </Scene>
  );
}
