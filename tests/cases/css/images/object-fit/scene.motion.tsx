export const composition = { width: 360, height: 150, fps: 1, duration: 1 };
export const controls = {
  resources: {
    picture: resource({ kind: "image", default: "/tests/resources/svg/pixels.png" }),
  },
};

export default function Scene() {
  return (
    <View style={{ width: 360, height: 150, backgroundColor: "white" }}>
      <Image
        key="contain"
        src="resource://picture"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 100,
          height: 100,
          objectFit: "contain",
          imageRendering: "pixelated",
          backgroundColor: "#eef2f7",
        }}
      />
      <Image
        key="cover"
        src="resource://picture"
        style={{
          position: "absolute",
          left: 130,
          top: 20,
          width: 100,
          height: 100,
          objectFit: "cover",
          imageRendering: "pixelated",
          backgroundColor: "#eef2f7",
        }}
      />
      <Image
        key="fill"
        src="resource://picture"
        style={{
          position: "absolute",
          left: 240,
          top: 20,
          width: 100,
          height: 100,
          objectFit: "fill",
          imageRendering: "pixelated",
          backgroundColor: "#eef2f7",
        }}
      />
    </View>
  );
}
