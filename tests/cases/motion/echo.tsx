export const composition = { width: 256, height: 64, fps: 4, duration: 2 };

// A dot that slides right with a trail of four copies 0.15 s apart, each older one fainter.
function Dot() {
  const { seconds } = useCurrentTime();
  return (
    <View
      style={{
        position: "absolute",
        left: interpolate(seconds, [0, 2], [8, 216]),
        top: 16,
        width: 32,
        height: 32,
        borderRadius: 16,
        background: "#e53935",
      }}
    />
  );
}

export default function Scene() {
  return (
    <Echo count={4} delay={0.15} decay={0.6}>
      <Dot />
    </Echo>
  );
}
