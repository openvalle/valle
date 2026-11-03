export const composition = { width: 256, height: 128, fps: 4, duration: 1 };

// A grid shaded by plane noise drifting across it, and a bar whose width follows line noise.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const cells = Array.from({ length: 64 }, (_, i) => [i % 16, Math.floor(i / 16)]);
  return (
    <View style={{ padding: 8 }}>
      <View style={{ display: "flex", flexWrap: "wrap", width: 240 }}>
        {cells.map(([x, y]) => (
          <View
            style={{
              width: 15,
              height: 15,
              background: "#1e88e5",
              opacity: noise2d(5, x * 0.35 + seconds, y * 0.35),
            }}
          />
        ))}
      </View>
      <View
        style={{
          marginTop: 8,
          height: 16,
          width: 8 + 232 * noise1d(5, seconds * 3),
          background: "#fb8c00",
        }}
      />
    </View>
  );
}
