export const composition = { width: 256, height: 320, fps: 1, duration: 1 };

// Ellipses: a line that does not wrap cut short, underlined to where the ellipsis starts; a Span
// of a larger size cut in the middle, which keeps its box; a long word in text that wraps; a
// centered line, which overflows to the left edge; spaced letters; text that fits; and text that
// does not clip, which keeps all of its line.
export default function Scene() {
  const row = {
    background: "#eceff1",
    marginBottom: 4,
    width: 120,
    fontSize: 14,
    overflow: "hidden",
    textOverflow: "ellipsis",
  };
  const line = { ...row, whiteSpace: "nowrap" };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={line}>The quick brown fox jumps</Text>
      <Text style={{ ...line, textDecoration: "underline #e53935" }}>
        The quick brown fox jumps
      </Text>
      <Text style={line}>
        The quick <Span style={{ color: "#1e88e5", fontSize: 18 }}>brown fox</Span> jumps
      </Text>
      <Text style={row}>Incomprehensibilities of words that wrap</Text>
      <Text style={{ ...line, textAlign: "center" }}>The quick brown fox jumps</Text>
      <Text style={{ ...line, letterSpacing: 2 }}>The quick brown fox jumps</Text>
      <Text style={line}>Short</Text>
      <Text style={{ ...line, overflow: "visible" }}>The quick brown fox jumps</Text>
    </View>
  );
}
