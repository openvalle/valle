export const composition = { width: 480, height: 330, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 480, height: 330, background: "white" }}>
      <Image src="resource://picture" style={{ width: 480, height: 330 }} />
    </View>
  );
}
