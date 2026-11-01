export const composition = { width: 300, height: 176, fps: 1, duration: 1 };

export const controls = {
  resources: {
    jpeg: resource({ kind: "image", default: "../resources/photo.jpg" }),
    lossy: resource({ kind: "image", default: "../resources/photo.webp" }),
    lossless: resource({ kind: "image", default: "../resources/tile.webp" }),
  },
};

// Images from JPEG and WebP files: a JPEG with its color subsampled, a lossy WebP and a lossless
// one with alpha, at their natural size and scaled by whole numbers, and the JPEG as a background.
export default function Scene() {
  return (
    <View
      style={{ display: "flex", flexWrap: "wrap", gap: 6, padding: 6, alignItems: "flex-start" }}
    >
      <Image src="resource://jpeg" />
      <Image src="resource://jpeg" style={{ width: 128 }} />
      <Image src="resource://lossy" />
      <Image src="resource://lossy" style={{ width: 128 }} />
      <Image src="resource://lossless" style={{ width: 32 }} />
      <View style={{ width: 64, height: 40, background: "url(resource://jpeg) center / 50%" }} />
    </View>
  );
}
