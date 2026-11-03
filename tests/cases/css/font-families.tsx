export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// The built-in families: serif and monospace beside sans-serif, at light and heavy weights,
// condensed, italic, and mixed in one paragraph; a list takes its first family.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, fontSize: 16 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={{ ...row, fontFamily: "sans-serif" }}>Sans-serif text</Text>
      <Text style={{ ...row, fontFamily: "serif" }}>Serif text, 1234</Text>
      <Text style={{ ...row, fontFamily: "monospace" }}>Mono: iiii WWWW</Text>
      <Text style={{ ...row, fontFamily: "serif", fontWeight: 300 }}>Light serif</Text>
      <Text style={{ ...row, fontFamily: "serif", fontWeight: 800 }}>Heavy serif</Text>
      <Text style={{ ...row, fontFamily: "monospace", fontWeight: 700, fontStretch: "condensed" }}>
        Bold condensed mono
      </Text>
      <Text style={{ ...row, fontFamily: "serif", fontStyle: "italic" }}>Italic serif</Text>
      <Text style={row}>
        Sans, <Span style={{ fontFamily: "serif", color: "#1e88e5" }}>serif</Span> and{" "}
        <Span style={{ fontFamily: "monospace", color: "#e53935" }}>mono</Span>
      </Text>
      <Text style={{ ...row, fontFamily: "monospace, serif" }}>First of a list</Text>
    </View>
  );
}
