export const composition = { width: 256, height: 340, fps: 1, duration: 1 };

// Backgrounds clipped to text: a gradient through the glyphs; through an underline too; a layer
// clipped to text over one that is not; a color, under a stroke and a shadow, which paint over
// it; and a box's through the text of its children and their spans, under a span in a color of
// its own and a child's own background.
export default function Scene() {
  const row = { marginBottom: 6, fontSize: 32, fontWeight: 800, color: "transparent" };
  const gradient = "linear-gradient(90deg, #e53935, #8e24aa, #1e88e5)";
  return (
    <View style={{ padding: 6 }}>
      <Text style={{ ...row, background: gradient, backgroundClip: "text" }}>Gradient</Text>
      <Text style={{ ...row, background: `${gradient} text`, textDecoration: "underline 4px" }}>
        Underlined
      </Text>
      <Text style={{ ...row, background: `${gradient} text, #ffe082` }}>Layered</Text>
      <Text
        style={{
          ...row,
          background: "#00897b text",
          WebkitTextStroke: "1px #263238",
          textShadow: "3px 3px rgb(0 0 0 / 30%)",
        }}
      >
        Stroked
      </Text>
      <View style={{ ...row, fontWeight: 400, background: `${gradient} text` }}>
        <Text style={{ fontSize: 24 }}>
          <Span style={{ fontSize: 32 }}>Child</Span> span
        </Text>
        <Text style={{ fontSize: 24 }}>
          <Span style={{ color: "#263238" }}>Own</Span> color
        </Text>
        <Text style={{ fontSize: 24, background: "#eceff1" }}>Covered</Text>
      </View>
    </View>
  );
}
