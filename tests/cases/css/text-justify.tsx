export const composition = { width: 256, height: 448, fps: 1, duration: 1 };

// Justified lines: full to both edges but for the last; with Spans of other sizes, an indent and
// letter and word spacing; before a forced break, and with no gap to widen.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, width: 200, textAlign: "justify" };
  const words = "The quick brown fox jumps over the lazy dog and runs into the woods";
  return (
    <View style={{ padding: 8, color: "#263238", fontSize: 13 }}>
      <Text style={row}>{words}</Text>
      <Text style={row}>
        {words} <Span style={{ fontSize: 18, color: "#1e88e5" }}>far away</Span> from home
      </Text>
      <Text style={{ ...row, textIndent: 24, letterSpacing: 1, wordSpacing: 2 }}>{words}</Text>
      <Text style={{ ...row, whiteSpace: "pre-line" }}>{`${words}\n${words}`}</Text>
      <Text style={{ ...row, width: 60 }}>Incomprehensibilities unbreak</Text>
      <Text style={{ ...row, textDecorationLine: "underline" }}>{words}</Text>
    </View>
  );
}
