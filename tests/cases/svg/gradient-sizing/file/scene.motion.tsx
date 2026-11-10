export const composition = { width: 320, height: 160, fps: 1, duration: 1 };
export const controls = {
  resources: {
    picture: resource({ kind: "image", default: "./source.svg" }),
  },
};

export default function Scene() {
  return (
    <View style={{ width: 320, height: 160, backgroundColor: "white" }}>
      <Image
        key="natural"
        src="resource://picture"
        style={{ position: "absolute", left: 20, top: 20 }}
      />
      <Image
        key="fit"
        src="resource://picture"
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 120,
          height: 120,
          objectFit: "contain",
        }}
      />
    </View>
  );
}
