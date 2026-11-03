export const composition = { width: 160, height: 90, fps: 4, duration: 1 };

// A transition: the next clip uncovered from the left, behind an edge that its progress moves.
export const role = transition();

export default function Split() {
  const { width, height } = useComposition();
  const { clipProgress } = useCurrentTime();
  return (
    <View style={{ width, height }}>
      <Image src="input://from" style={{ position: "absolute", inset: 0 }} />
      <View
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          width: width * clipProgress,
          height,
          overflow: "hidden",
          borderRight: "2px solid #facc15",
        }}
      >
        <Image src="input://to" style={{ position: "absolute", left: 0, top: 0, width, height }} />
      </View>
    </View>
  );
}
