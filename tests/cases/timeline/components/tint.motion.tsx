export const composition = { width: 160, height: 90, fps: 4, duration: 2 };

export const controls = {
  props: { hue: number({ default: 90, min: 0, max: 360 }) },
};

// An effect: the clip's picture with its hue turned by `hue` degrees.
export const role = effect();

export default function Tint({ hue }) {
  const { width, height } = useComposition();
  return (
    <View style={{ width, height }}>
      <Image
        src="input://source"
        style={{ position: "absolute", inset: 0, filter: `hue-rotate(${hue}deg)` }}
      />
    </View>
  );
}
