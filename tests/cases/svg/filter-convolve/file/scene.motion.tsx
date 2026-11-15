export const composition = { width: 450, height: 130, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 450, height: 130, background: "white" }}>
      <Image src="resource://picture" style={{ width: 450, height: 130 }} />
    </View>
  );
}
