export const composition = { width: 480, height: 300, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Latin: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }),
    picture: resource({ kind: "image", default: "./source.svg" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 480, height: 300, backgroundColor: "white" }}>
      <Image src="resource://picture" style={{ width: 480, height: 300 }} />
    </View>
  );
}
