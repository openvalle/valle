export const composition = { width: 300, height: 40, fps: 1, duration: 1 };

export const controls = {
  resources: {
    wide: resource({ kind: "image", default: "../resources/wide.png" }),
  },
};

// An image 9000 px wide, wider than a GPU's textures need to go: its texture starts at the first
// level of its mipmap that fits.
export default function Scene() {
  return (
    <View style={{ padding: 6 }}>
      <Image src="resource://wide" style={{ width: 288 }} />
    </View>
  );
}
