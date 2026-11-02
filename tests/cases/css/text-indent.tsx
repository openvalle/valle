export const composition = { width: 256, height: 448, fps: 1, duration: 1 };

// First lines indented: by a length, by a percentage of the width, by a negative length out of
// the box, as a parent gives it, centered and right-aligned, balanced, in a box sized to its
// content, and only the first line after a forced break.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, width: 200 };
  const words = "The quick brown fox jumps over the lazy dog and runs into the woods";
  return (
    <View style={{ padding: 8, color: "#263238", fontSize: 13 }}>
      <Text style={{ ...row, textIndent: 24 }}>{words}</Text>
      <Text style={{ ...row, textIndent: "25%" }}>{words}</Text>
      <Text style={{ ...row, textIndent: -6, marginLeft: 8 }}>{words}</Text>
      <View style={{ ...row, textIndent: "2em" }}>
        <Text style={{ fontSize: 10 }}>
          {words} <Span style={{ fontSize: 16, color: "#1e88e5" }}>far away</Span>
        </Text>
      </View>
      <Text style={{ ...row, textIndent: 24, textAlign: "center" }}>{words}</Text>
      <Text style={{ ...row, textIndent: "calc(10% + 1em)", textAlign: "right" }}>{words}</Text>
      <Text style={{ ...row, textIndent: 24, textWrap: "balance" }}>{words}</Text>
      <View style={{ display: "flex", marginBottom: 4 }}>
        <Text style={{ background: "#eceff1", textIndent: 16 }}>Fits its box</Text>
      </View>
      <Text style={{ ...row, textIndent: 16, whiteSpace: "pre-line" }}>{`${words}\n${words}`}</Text>
    </View>
  );
}
