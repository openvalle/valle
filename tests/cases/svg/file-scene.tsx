export const composition = { width: 400, height: 280, fps: 1, duration: 1 };

export const controls = {
  resources: {
    scene: resource({ kind: "image", default: "../resources/scene.svg" }),
  },
};

// A whole drawing as an SVG file: gradients, a clip path, a mask, a symbol with its copies, markers,
// style attributes and inherited paint, a small embedded image read at its nearest px, and the
// title, comments and metadata editors leave in. At its natural size, twice it, and fit into boxes
// of other shapes.
export default function Scene() {
  const image = { background: "#eceff1" };
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8 }}>
      <Image src="resource://scene" style={image} />
      <Image src="resource://scene" style={{ ...image, width: 240 }} />
      <Image
        src="resource://scene"
        style={{ ...image, width: 90, height: 60, objectFit: "contain" }}
      />
      <Image
        src="resource://scene"
        style={{ ...image, width: 90, height: 60, objectFit: "cover" }}
      />
      <Image src="resource://scene" style={{ ...image, width: 90, height: 60, opacity: 0.6 }} />
    </View>
  );
}
