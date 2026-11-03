export const composition = { width: 256, height: 48, fps: 4, duration: 2 };

// A bar that grows, holds from half a second to one, and grows on from where it stopped.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const t = freezeTime(seconds, { from: 0.5, to: 1 });
  return (
    <View style={{ padding: 8 }}>
      <View
        style={{ height: 16, width: interpolate(t, [0, 1.5], [8, 200]), background: "#1e88e5" }}
      />
      <View
        style={{
          height: 8,
          marginTop: 4,
          width: interpolate(seconds, [0, 2], [8, 200]),
          background: "#b0bec5",
        }}
      />
    </View>
  );
}
