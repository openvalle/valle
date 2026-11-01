export const composition = { width: 256, height: 320, fps: 1, duration: 1 };

// Text shadows: a hard one; a glow; two, the first on top; one in currentcolor; offsets in em;
// a Span with its own over the Text's, and an underline with its shadow; and text with no fill
// that only its blurred shadow shows.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 6, fontSize: 24 };
  return (
    <View style={{ padding: 6, color: "#263238" }}>
      <Text style={{ ...row, textShadow: "2px 2px #e53935" }}>Hard shadow</Text>
      <Text style={{ ...row, color: "#fff", textShadow: "0 0 6px #1e88e5" }}>Glowing</Text>
      <Text style={{ ...row, textShadow: "3px 3px 0 #00897b, -3px -3px 2px #8e24aa" }}>
        Two shadows
      </Text>
      <Text style={{ ...row, color: "rgb(38 50 56 / 40%)", textShadow: "3px 4px" }}>
        Current color
      </Text>
      <Text style={{ ...row, textShadow: "0.1em 0.1em 0.2em #e53935" }}>
        In em{" "}
        <Span
          style={{ color: "#1e88e5", textShadow: "-2px 3px #ffca28", textDecoration: "underline" }}
        >
          span
        </Span>
      </Text>
      <Text style={{ ...row, WebkitTextFillColor: "transparent", textShadow: "0 0 3px #263238" }}>
        Blurred away
      </Text>
    </View>
  );
}
