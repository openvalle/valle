export const composition = { width: 160, height: 90, fps: 4, duration: 1 };

// An adjustment: all under it in gray, and a bar that runs down it.
export const role = effect();

export default function Scan() {
  const { width, height } = useComposition();
  const { clipProgress } = useCurrentTime();
  return (
    <View style={{ width, height }}>
      <Image
        src="input://source"
        style={{ position: "absolute", inset: 0, filter: "grayscale(1)" }}
      />
      <View
        style={{
          position: "absolute",
          left: 0,
          top: (height - 6) * clipProgress,
          width,
          height: 6,
          background: "rgba(34, 211, 238, 0.7)",
        }}
      />
    </View>
  );
}
