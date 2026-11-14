export const composition = { width: 360, height: 260, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 360, height: 260, backgroundColor: "white" }}>
      <Image src="resource://picture" style={{ width: 360, height: 260 }} />
    </View>
  );
}
