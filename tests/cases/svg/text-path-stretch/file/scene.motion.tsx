export const composition = { width: 420, height: 320, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Latin: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    picture: resource({ kind: "image", default: "./source.svg" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 420, height: 320, backgroundColor: "white" }}>
      <Image src="resource://picture" style={{ width: 420, height: 320 }} />
    </View>
  );
}
