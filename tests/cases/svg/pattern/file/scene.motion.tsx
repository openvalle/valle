export const composition = { width: 300, height: 210, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 300, height: 210, backgroundColor: "white" }}>
      <Image src="resource://picture" style={{ width: 300, height: 210 }} />
    </View>
  );
}
