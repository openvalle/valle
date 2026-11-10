export const composition = { width: 380, height: 150, fps: 1, duration: 1 };
export const controls = {
  resources: {
    picture: resource({ kind: "image", default: "/tests/resources/svg/pixels.png" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 380, height: 150, backgroundColor: "white" }}>
      <Image
        key="left"
        src="resource://picture"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 100,
          height: 100,
          objectFit: "cover",
          objectPosition: "left center",
          imageRendering: "pixelated",
        }}
      />
      <Image
        key="center"
        src="resource://picture"
        style={{
          position: "absolute",
          left: 140,
          top: 20,
          width: 100,
          height: 100,
          objectFit: "cover",
          objectPosition: "center",
          imageRendering: "pixelated",
        }}
      />
      <Image
        key="right"
        src="resource://picture"
        style={{
          position: "absolute",
          left: 260,
          top: 20,
          width: 100,
          height: 100,
          objectFit: "cover",
          objectPosition: "right center",
          imageRendering: "pixelated",
        }}
      />
    </View>
  );
}
