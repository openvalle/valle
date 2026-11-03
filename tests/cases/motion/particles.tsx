export const composition = { width: 256, height: 128, fps: 4, duration: 2.5 };

// Sparks from a slot at the bottom left, looping: falling free on the left, swirled by curl noise
// and slowed by drag on the right; each fades and shrinks over its life. Below, bars filling to
// their heights one after another from the middle with field().
export default function Scene() {
  const { seconds } = useCurrentTime();
  const spark = { seed: 4, count: 24, birth: { interval: 0.1 }, lifetime: 1.2, loop: true };
  const free = particles(seconds, {
    ...spark,
    emitter: { x: 40, y: 80, width: 24, height: 2 },
    velocity: { x: [-30, 30], y: [-120, -80] },
    gravity: { x: 0, y: 120 },
  });
  const swirled = particles(seconds, {
    ...spark,
    emitter: { x: 170, y: 80, width: 24, height: 2 },
    velocity: { x: [-30, 30], y: [-120, -80] },
    forces: [curlNoise({ seed: 3, scale: 0.03, strength: 900 }), drag(1.5)],
  });
  const dot = (p, color) => (
    <View
      key={p.key}
      style={{
        position: "absolute",
        left: p.x,
        top: p.y,
        width: 8 - 5 * p.progress,
        height: 8 - 5 * p.progress,
        borderRadius: 4,
        background: color,
        opacity: p.alive ? 1 - p.progress : 0,
      }}
    />
  );
  const heights = field({
    from: [0, 0, 0, 0, 0],
    to: [14, 22, 30, 22, 14],
    progress: clamp(seconds / 2, 0, 1),
    stagger: [0.4, 0.2, 0, 0.2, 0.4],
  });
  return (
    <View style={{ position: "absolute", inset: 0, background: "#0f172a" }}>
      {free.map((p) => dot(p, "#fbbf24"))}
      {swirled.map((p) => dot(p, "#38bdf8"))}
      <View
        style={{
          position: "absolute",
          left: 96,
          bottom: 4,
          display: "flex",
          gap: 4,
          alignItems: "flex-end",
        }}
      >
        {heights.map((h, i) => (
          <View key={i} style={{ width: 8, height: h, background: "#a78bfa" }} />
        ))}
      </View>
    </View>
  );
}
