export const composition = { width: 400, height: 120, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 400, height: 120, backgroundColor: "white" }}>
      <Image src="resource://picture" style={{ width: 400, height: 120 }} />
    </View>
  );
}
