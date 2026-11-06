export const composition = { width: 160, height: 90, fps: 4, duration: 2 };

// An effect that paints past the clip's box: a white glow around the clip's picture.
export const role = effect();

export default function Glow() {
  const { width, height } = useComposition();
  return (
    <View style={{ width, height }}>
      <Image
        src="input://source"
        style={{ position: "absolute", inset: 0, filter: "drop-shadow(0 0 4px white)" }}
      />
    </View>
  );
}
