export const composition = { width: 380, height: 230, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 380, height: 230, background: "white" }}>
      <Image src="resource://picture" style={{ width: 380, height: 230 }} />
    </View>
  );
}
