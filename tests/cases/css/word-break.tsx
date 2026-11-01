export const composition = { width: 256, height: 384, fps: 1, duration: 1 };

// Breaking words: a long word that overflows its line, then one broken by word-break: break-all,
// overflow-wrap: anywhere and break-word, and word-break: break-word; a Span that breaks all in
// text that does not (digits: parley keeps a kerning pair split across lines, which Chrome
// drops); and flex items that shrink to their min-content width, which anywhere breaks words
// for and break-word does not.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, width: 120 };
  const word = "Incomprehensibilities of words";
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={row}>{word}</Text>
      <Text style={{ ...row, wordBreak: "break-all" }}>{word}</Text>
      <Text style={{ ...row, overflowWrap: "anywhere" }}>{word}</Text>
      <Text style={{ ...row, overflowWrap: "break-word" }}>{word}</Text>
      <Text style={{ ...row, wordBreak: "break-word" }}>{word}</Text>
      <Text style={row}>
        Only{" "}
        <Span style={{ wordBreak: "break-all", color: "#e53935" }}>31415926535897932384626433</Span>{" "}
        here
      </Text>
      <View style={{ display: "flex", width: 150, gap: 4 }}>
        <Text style={{ background: "#ffe0b2", overflowWrap: "anywhere" }}>Unbreakable</Text>
        <Text style={{ background: "#c8e6c9", overflowWrap: "break-word" }}>Unbreakable</Text>
      </View>
    </View>
  );
}
