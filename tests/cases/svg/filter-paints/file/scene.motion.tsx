export const composition = { width: 440, height: 240, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 440, height: 240, backgroundColor: "white" }}>
      <Image src="resource://picture" style={{ width: 440, height: 240 }} />
    </View>
  );
}
