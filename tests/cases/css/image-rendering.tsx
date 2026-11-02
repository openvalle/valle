export const composition = { width: 320, height: 256, fps: 1, duration: 1 };

export const controls = {
  resources: {
    photo: resource({ kind: "image", default: "../resources/photo.png" }),
    rings: resource({ kind: "image", default: "../resources/rings.png" }),
  },
};

// Images read at their nearest px, scaled up by whole and fractional factors, and as `auto` reads
// them; scaled down; inherited, and taken back; as a tiled background and as a mask.
export default function Scene() {
  const pixelated = { imageRendering: "pixelated" };
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      <Image src="resource://photo" style={{ ...pixelated, width: 128 }} />
      <Image src="resource://photo" style={{ width: 128 }} />
      <Image src="resource://photo" style={{ ...pixelated, width: 92, height: 60 }} />
      <Image src="resource://rings" style={{ ...pixelated, width: 48 }} />
      <Image src="resource://rings" style={{ ...pixelated, width: 96, height: 32 }} />
      <View style={{ ...pixelated, display: "flex", gap: 4 }}>
        <Image src="resource://photo" style={{ width: 50 }} />
        <Image src="resource://photo" style={{ width: 50, imageRendering: "auto" }} />
      </View>
      <View
        style={{
          ...pixelated,
          width: 80,
          height: 56,
          backgroundImage: "url(resource://photo)",
          backgroundSize: "50px 34px",
        }}
      />
      <View
        style={{
          ...pixelated,
          width: 64,
          height: 64,
          background: "#3949ab",
          maskImage: "url(resource://rings)",
          maskSize: "100% 100%",
        }}
      />
    </View>
  );
}
