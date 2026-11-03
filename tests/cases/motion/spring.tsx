export const composition = { width: 256, height: 136, fps: 4, duration: 1 };

// Springs moving dots across: the default, a preset, SwiftUI's duration and bounce, an
// overdamped one with mass, one thrown back at the start, and one stretched by its velocity.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const dot = (color, options, stretch) => (
    <View
      style={{
        width: 16,
        height: 16,
        borderRadius: 8,
        background: color,
        translate: `${spring(seconds, options) * 200}px 0px`,
        scale: stretch ? `${1 + springVelocity(seconds, options) / 20} 1` : "1",
      }}
    />
  );
  return (
    <View style={{ display: "flex", flexDirection: "column", padding: 8, gap: 4 }}>
      {dot("#1e88e5", {})}
      {dot("#43a047", { preset: "wobbly" })}
      {dot("#fb8c00", { duration: 0.6, bounce: 0.3 })}
      {dot("#e53935", { mass: 2, stiffness: 120, damping: 40 })}
      {dot("#8e24aa", { preset: "bouncy", initialVelocity: -8 })}
      {dot("#00897b", { preset: "stiff" }, true)}
    </View>
  );
}
