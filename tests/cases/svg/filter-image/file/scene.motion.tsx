export const composition = { width: 520, height: 140, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 520, height: 140, background: "white" }}>
      <Image src="resource://picture" style={{ width: 520, height: 140 }} />
    </View>
  );
}
