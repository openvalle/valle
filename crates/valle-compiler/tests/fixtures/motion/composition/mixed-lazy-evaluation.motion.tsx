export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const DOTS = Array.from({ length: 64 }, (_, i) => ({
  id: `d${i}`, i, x: 4 + (i % 8) * 8, y: 4 + Math.floor(i / 8) * 14
}));
const MARK = path("M 0 0 L 20 0 L 20 4 Z");

export default function Mixed(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <Path key="marker" d={MARK} fill="#ff0000"
      style={{ opacity: bounds("d20").width / 4 }} />
    {DOTS.map((dot) => (
      <View key={dot.id} className="absolute" visible={ctx.seconds * 1000 >= dot.i}
        style={{ left: dot.x, top: dot.y, width: 4, height: 4, backgroundColor: "#ffffff",
          opacity: clamp(ctx.seconds * 1000 - dot.i, 0, 1) }} />
    ))}
  </Scene>;
}
