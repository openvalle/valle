export const composition = { width: 784, height: 676, fps: 1, duration: 1 };
export const controls = {
  resources: {
    picture: resource({ kind: "image", default: "./source.svg" }),
    Test: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 784, height: 676, background: "white" }}>
      <Image src="resource://picture" style={{ width: 784, height: 676 }} />
    </View>
  );
}
