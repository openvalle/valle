export const composition = { width: 320, height: 120, fps: 30, duration: 2 };
const CELLS = Array.from({ length: 4 }, (_, i) => ({ id: `c${i}`, x: 20 + i * 30, delay: i * 0.1 }));
export default function SampledInstances(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <Shutter key="shutter" samples={4} angle={180}>
      {CELLS.map((cell) => (
        <View key={cell.id} className="absolute" style={{ left: cell.x, top: 20, width: 16, height: 16,
          backgroundColor: "#ffffff", opacity: clamp(ctx.seconds * 2 - cell.delay, 0, 1) }} />
      ))}
    </Shutter>
  </Scene>;
}
