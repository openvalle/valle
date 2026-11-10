export const composition = { width: 340, height: 220, fps: 1, duration: 1 };
export const controls = {
  resources: {
    picture: resource({ kind: "image", default: "./source.svg" }),
  },
};

export default function Scene() {
  return (
    <View style={{ width: 340, height: 220, backgroundColor: "white" }}>
      <Image key="drawing" src="resource://picture" style={{ width: 340, height: 220 }} />
    </View>
  );
}
