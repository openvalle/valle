export const composition = { width: 256, height: 128, fps: 4, duration: 2 };

const timing = defineSequence({
  enter: { duration: 0.5 },
  rise: { after: "enter", overlap: 0.25, duration: 1 },
  pulse: { at: 0.5, duration: 1.5 },
});

// A sequence driving bars: one stage, one that overlaps it, dots staggered through a stage, one
// that repeats and one that goes there and back.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const bar = (color, p) => <View style={{ height: 12, width: 8 + 200 * p, background: color }} />;
  return (
    <View style={{ display: "flex", flexDirection: "column", padding: 8, gap: 6 }}>
      {bar("#1e88e5", stageProgress(seconds, timing.enter))}
      {bar("#43a047", stageProgress(seconds, timing.rise))}
      <View style={{ display: "flex", gap: 6 }}>
        {[0, 1, 2, 3, 4].map((i) => (
          <View
            style={{
              width: 24,
              height: 24,
              background: "#fb8c00",
              opacity: staggerProgress(seconds, timing.rise, i, 0.15),
            }}
          />
        ))}
      </View>
      {bar("#e53935", repeatProgress(seconds, timing.pulse, 3))}
      {bar("#8e24aa", yoyoProgress(seconds, timing.pulse, 2))}
    </View>
  );
}
