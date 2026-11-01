export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Lines broken to fit, alignment, sizes, weights, line heights and letter spacing, and text sizing
// the boxes around it.
export default function Scene() {
  const text = "The quick brown fox jumps over the lazy dog";
  return (
    <View style={{ padding: 6, fontSize: 12 }}>
      <View style={{ width: 120, background: "#eeeeee" }}>
        <Text>{text}</Text>
      </View>
      <View style={{ width: 150, marginTop: 4, background: "#e0e0e0" }}>
        <Text style={{ textAlign: "center", fontSize: 14, lineHeight: 1.425 }}>{text}</Text>
      </View>
      <View style={{ width: 140, marginTop: 4, background: "#eeeeee" }}>
        <Text
          style={{ textAlign: "right", fontWeight: 700, lineHeight: "16.9px", letterSpacing: 1 }}
        >
          {text}
        </Text>
      </View>
      <View style={{ display: "flex", marginTop: 4, gap: 4, background: "#e0e0e0" }}>
        <Text style={{ fontSize: 18 }}>Fit</Text>
        <Text style={{ fontSize: 10 }}>{text}</Text>
      </View>
      <View
        style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", marginTop: 4 }}
      >
        <Text style={{ fontSize: 20, fontWeight: 300, background: "#bdbdbd" }}>Light 20</Text>
        <Text style={{ fontSize: 9, letterSpacing: "0.2em", background: "#e0e0e0" }}>
          {" "}
          spaced out{" "}
        </Text>
      </View>
    </View>
  );
}
