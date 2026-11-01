export const composition = { width: 300, height: 150, fps: 1, duration: 1 };

export const controls = {
  resources: {
    rings: resource({ kind: "image", default: "../resources/rings.png" }),
  },
};

// Rings finer and finer out to the edge, scaled down: by 2, 3.2, 4 and 6.4, repeated in tiles 24
// px wide, and turned; each pixel reads the level of the image's mipmap Chrome reads.
export default function Scene() {
  return (
    <View
      style={{ display: "flex", flexWrap: "wrap", gap: 6, padding: 6, alignItems: "flex-start" }}
    >
      <Image src="resource://rings" style={{ width: 64 }} />
      <Image src="resource://rings" style={{ width: 40 }} />
      <Image src="resource://rings" style={{ width: 32 }} />
      <Image src="resource://rings" style={{ width: 20 }} />
      <View style={{ width: 80, height: 64, background: "url(resource://rings) 0 0 / 24px" }} />
      <Image src="resource://rings" style={{ width: 48, rotate: "30deg" }} />
    </View>
  );
}
