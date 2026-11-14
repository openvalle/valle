export const composition = { width: 320, height: 220, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 320, height: 220, backgroundColor: "#dae9dd" }}>
      <Image src="resource://picture" style={{ width: 320, height: 220 }} />
    </View>
  );
}
