export const composition = { width: 256, height: 128, fps: 4, duration: 1 };

// Bars growing by each kind of easing, frame by frame: linear, a named curve, a Bézier that goes
// past its end, steps and a bounce written with linear().
export default function Scene() {
  const { seconds } = useCurrentTime();
  const bar = (color, easing, options) => (
    <View
      style={{
        height: 16,
        marginBottom: 4,
        background: color,
        width: interpolate(seconds, [0, 1], [8, 200], { easing, ...options }),
      }}
    />
  );
  return (
    <View style={{ padding: 8 }}>
      {bar("#1e88e5", "linear")}
      {bar("#43a047", "ease-in-out")}
      {bar("#fb8c00", "cubic-bezier(0.34, 1.56, 0.64, 1)")}
      {bar("#e53935", "steps(3, jump-both)")}
      {bar("#8e24aa", "linear(0, 0.3 20%, 1 50%, 0.7 70%, 1)")}
    </View>
  );
}
