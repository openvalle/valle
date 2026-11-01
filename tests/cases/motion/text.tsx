export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Sizes, weights, colors, alignment and wrapping in the built-in sans-serif.
export default function Scene() {
  return (
    <View style={{ height: 256, padding: 8, background: "white", color: "#212121" }}>
      <Text style={{ fontSize: 12 }}>Twelve pixels, regular</Text>
      <Text>Sixteen pixels by default</Text>
      <Text style={{ fontSize: 24, fontWeight: 700 }}>Bold 24 px</Text>
      <Text style={{ fontSize: 40, color: "#1e88e5", letterSpacing: -1 }}>Valle</Text>
      <Text style={{ textAlign: "center", lineHeight: 1.5 }}>
        Centered text that wraps onto a second line
      </Text>
      <View style={{ background: "#263238", padding: 8, marginTop: 8 }}>
        <Text style={{ color: "white", fontWeight: 300 }}>Light text on a dark ground</Text>
      </View>
    </View>
  );
}
