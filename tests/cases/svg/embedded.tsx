export const composition = { width: 412, height: 198, fps: 1, duration: 1 };

export const controls = {
  resources: {
    badge: resource({ kind: "image", default: "../resources/badge.svg" }),
  },
};

// Images an SVG file embeds as data URIs: a PNG in a viewport of another ratio, its data over
// several lines; a JPEG sliced at the bottom left; a WebP stretched, translucent and clipped to a
// circle in fractions of its viewport; copies of a PNG, one turned; and a WebP at its natural
// size. The file at its natural size and twice it.
export default function Scene() {
  return (
    <View
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 6,
        padding: 6,
        alignItems: "flex-start",
        background: "#eceff1",
      }}
    >
      <Image src="resource://badge" />
      <Image src="resource://badge" style={{ width: 400 }} />
    </View>
  );
}
