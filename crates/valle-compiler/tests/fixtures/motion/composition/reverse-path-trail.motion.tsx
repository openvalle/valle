export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const ORBIT = arc(point(320, 180), 140, 0, TAU);
const TAIL = defineRepeater({ count: 10, keyPrefix: "tail" });
export default function Reverse(ctx) {
  const base = fract(ctx.seconds * 0.4);
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      {TAIL.map((tail) => (
        <View key={tail.key} className="absolute" style={{ width: 12, height: 12, borderRadius: 6, backgroundColor: "#ffffff",
          opacity: 1 - tail.progress,
          motionPath: follow(reversePath(ORBIT), trail(base, tail.index, { gap: -0.01, mode: "wrap" }), { rotate: "none" }) }} />
      ))}
    </Scene>
  );
}
