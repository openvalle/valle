export const composition = { width: 256, height: 448, fps: 1, duration: 1 };

// Balanced lines: a heading of 2 to 6 lines centered, right-aligned and in Spans of other sizes;
// lines after the first forced break, which break as usual; and text of more than 6 lines, which
// does not balance.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, width: 200, textWrap: "balance" };
  const words = "The quick brown fox jumps over the lazy dog and runs into the woods";
  return (
    <View style={{ padding: 4, color: "#263238", fontSize: 13 }}>
      <Text style={{ ...row, fontSize: 18, textAlign: "center" }}>
        The quick brown <Span style={{ color: "#1e88e5" }}>fox jumps</Span> over the lazy dog and
        runs into the woods
      </Text>
      <Text style={{ ...row, textAlign: "right" }}>
        {words} <Span style={{ fontSize: 18, color: "#e53935" }}>far away</Span> from home
      </Text>
      <Text style={{ ...row, whiteSpace: "pre-line" }}>{`${words}\n${words}`}</Text>
      <Text style={{ ...row, width: 120, fontSize: 11 }}>
        {words} {words} {words}
      </Text>
    </View>
  );
}
