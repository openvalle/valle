export const composition = { width: 340, height: 180, fps: 1, duration: 1 };
export const controls = {
  resources: {
    picture: resource({ kind: "image", default: "./source.svg" }),
  },
};

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <Image key="drawing" src="resource://picture" style={{ width: 340, height: 180 }} />
    </View>
  );
}
