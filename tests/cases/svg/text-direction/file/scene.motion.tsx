export const composition = { width: 560, height: 360, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Latin: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    Hebrew: resource({ kind: "font", default: "/tests/resources/NotoSansHebrew.ttf" }),
    picture: resource({ kind: "image", default: "./source.svg" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 560, height: 360, backgroundColor: "white" }}>
      <Image src="resource://picture" style={{ width: 560, height: 360 }} />
    </View>
  );
}
