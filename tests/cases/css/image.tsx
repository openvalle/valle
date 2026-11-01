export const composition = { width: 256, height: 200, fps: 1, duration: 1 };

export const controls = {
  resources: {
    photo: resource({ kind: "image", default: "../resources/photo.png" }),
    tile: resource({ kind: "image", default: "../resources/tile.png" }),
  },
};

// Images: at their natural size; a width or a height alone, the other side as the natural ratio
// has it; both, stretched; scaled up, read between pixels, translucent ones too; with a border,
// padding and rounded corners that clip the image to the content box; stretched across a column;
// and turned, translucent, at whole px: Chrome measures a turned box at half a px around an
// origin it does not snap, though it draws it around one it does.
export default function Scene() {
  return (
    <View
      style={{ display: "flex", flexWrap: "wrap", gap: 6, padding: 6, alignItems: "flex-start" }}
    >
      <Image src="resource://photo" />
      <Image src="resource://photo" style={{ width: 96 }} />
      <Image src="resource://photo" style={{ height: 30 }} />
      <Image src="resource://photo" style={{ width: 50, height: 50 }} />
      <Image src="resource://tile" style={{ width: 80 }} />
      <Image
        src="resource://photo"
        style={{
          width: 94,
          border: "3px solid #3949ab",
          padding: 4,
          borderRadius: 16,
          background: "#e8eaf6",
        }}
      />
      <View style={{ display: "flex", flexDirection: "column", width: 40 }}>
        <Image src="resource://tile" />
      </View>
      <Image src="resource://photo" style={{ width: 64, rotate: "15deg", opacity: 0.8 }} />
    </View>
  );
}
