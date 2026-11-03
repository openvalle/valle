export const composition = { width: 256, height: 80, fps: 4, duration: 2 };

// Bars driven by clamp, by mod running past its length and through negative time, and by
// pingPong going there and back.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const bar = (color, p) => <View style={{ height: 16, width: 8 + 200 * p, background: color }} />;
  return (
    <View style={{ display: "flex", flexDirection: "column", padding: 8, gap: 4 }}>
      {bar("#1e88e5", clamp(seconds * 1.5 - 0.5, 0, 1))}
      {bar("#43a047", mod(seconds - 1.6, 0.75) / 0.75)}
      {bar("#fb8c00", pingPong(seconds, 0.8) / 0.8)}
    </View>
  );
}
