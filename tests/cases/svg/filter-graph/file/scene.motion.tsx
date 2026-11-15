export const composition = { width: 480, height: 360, fps: 4, duration: 2 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 480, height: 360, background: "white" }}>
      <Image src="resource://picture" style={{ width: 480, height: 360 }} />
    </View>
  );
}
