export const composition = { width: 256, height: 320, fps: 1, duration: 1 };

// Font styles, slanted as Chrome slants a font with no italic: italic; oblique, and oblique by
// 30deg, which leans no further; oblique by 10deg and by -20deg, which stay upright; an underline
// that skips the slanted descenders; heavy and condensed italics; an italic Span; and an italic
// ellipsis.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, fontSize: 18 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={{ ...row, fontStyle: "italic" }}>Italic style</Text>
      <Text style={{ ...row, fontStyle: "oblique" }}>Oblique style</Text>
      <Text style={{ ...row, fontStyle: "oblique 30deg" }}>Oblique by 30deg</Text>
      <Text style={{ ...row, fontStyle: "oblique 10deg" }}>Oblique by 10deg</Text>
      <Text style={{ ...row, fontStyle: "oblique -20deg" }}>Oblique by -20deg</Text>
      <Text style={{ ...row, fontStyle: "italic", textDecoration: "underline" }}>
        Gypsy jogging
      </Text>
      <Text style={{ ...row, fontStyle: "italic", fontWeight: 800, fontStretch: "condensed" }}>
        Heavy condensed italic
      </Text>
      <Text style={row}>
        Upright <Span style={{ fontStyle: "italic", color: "#1e88e5" }}>italic</Span>
      </Text>
      <Text
        style={{
          ...row,
          width: 150,
          fontStyle: "italic",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        The quick brown fox jumps
      </Text>
    </View>
  );
}
