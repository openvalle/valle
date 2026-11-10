export const composition = { width: 200, height: 120, fps: 1, duration: 1 };
export const controls = {
  resources: {
    picture: resource({ kind: "image", default: "./source.svg" }),
  },
};

export default function Scene() {
  return (
    <View style={{ width: 200, height: 120, backgroundColor: "white" }}>
      <Image
        key="image"
        src="resource://picture"
        style={{ position: "absolute", left: 20, top: 20, width: 160, height: 80 }}
      />
    </View>
  );
}
