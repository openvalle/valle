export const composition = { width: 360, height: 180, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 360, height: 180, background: "white" }}>
      <Image src="resource://picture" style={{ width: 360, height: 180 }} />
    </View>
  );
}
