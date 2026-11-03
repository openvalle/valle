export const composition = { width: 256, height: 96, fps: 4, duration: 2 };

// Squares drifting: one up and down, one in two dimensions, one turning; each its own seed.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const p = wiggle2D(seconds, { seed: 2, frequency: 1.5, amplitude: [24, 12] });
  const square = (color, style) => (
    <View style={{ width: 24, height: 24, background: color, ...style }} />
  );
  return (
    <View style={{ display: "flex", gap: 48, padding: 36 }}>
      {square("#1e88e5", {
        translate: `0px ${wiggle(seconds, { seed: 1, frequency: 2, amplitude: 24 })}px`,
      })}
      {square("#43a047", { translate: `${p.x}px ${p.y}px` })}
      {square("#fb8c00", {
        rotate: `${wiggle(seconds, { seed: 3, frequency: 1, amplitude: 90, phase: 0.5 })}deg`,
      })}
    </View>
  );
}
