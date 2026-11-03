export const composition = { width: 160, height: 90, fps: 4, duration: 1 };

// A badge that slides in for 0.25 s, holds, and slides out for 0.25 s, at any clip length.
export const role = overlay({ intro: 0.25, outro: 0.25, hold: "once" });

export default function Badge() {
  const { width } = useComposition();
  const { seconds } = useCurrentTime();
  const x = interpolate(seconds, [0, 0.25, 0.75, 1], [-60, 8, 8, -60]);
  return (
    <View
      style={{
        position: "absolute",
        left: x,
        top: 8,
        width: width / 4,
        height: 16,
        borderRadius: 8,
        background: "#22c55e",
      }}
    />
  );
}
