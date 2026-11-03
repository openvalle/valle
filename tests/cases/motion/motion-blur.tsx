export const composition = { width: 256, height: 96, fps: 4, duration: 1 };

// Two bars that cross the frame at 160 px a second, blurred over half of each frame: a trail
// of 20 px, along x and, in a turned box, along its own turned x.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const bar = {
    position: "absolute",
    left: 24 + seconds * 160,
    width: 16,
    height: 24,
    borderRadius: 4,
    motionBlur: { velocity: { x: 160, y: 0 } },
  };
  return (
    <View style={{ width: 256, height: 96, background: "#e3f2fd" }}>
      <View style={{ ...bar, top: 12, background: "#1e88e5" }} />
      <View
        style={{ position: "absolute", left: 0, top: 40, width: 256, height: 56, rotate: "10deg" }}
      >
        <View style={{ ...bar, top: 12, background: "#f4511e" }} />
      </View>
    </View>
  );
}
