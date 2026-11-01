export const composition = { width: 256, height: 320, fps: 1, duration: 1 };

// Text strokes: an outline with no fill; a stroke over a fill; a wide translucent one, its
// corners mitered or beveled; one in em, in each box's currentcolor; one with an underline in
// its color and a translucent shadow, one for stroke and fill; and the width keywords.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 6, fontSize: 28, color: "#263238" };
  return (
    <View style={{ padding: 6 }}>
      <Text style={{ ...row, WebkitTextFillColor: "transparent", WebkitTextStroke: "1px" }}>
        Outlined
      </Text>
      <Text style={{ ...row, WebkitTextFillColor: "#ffca28", WebkitTextStroke: "2px #1e88e5" }}>
        Stroked
      </Text>
      <Text
        style={{
          ...row,
          fontSize: 44,
          fontWeight: 800,
          WebkitTextStroke: "6px rgb(229 57 53 / 50%)",
        }}
      >
        AVWk
      </Text>
      <Text style={{ ...row, color: "#1e88e5", WebkitTextStroke: "0.05em" }}>
        In em <Span style={{ color: "#e53935" }}>span</Span>
      </Text>
      <Text
        style={{
          ...row,
          WebkitTextFillColor: "#fff",
          WebkitTextStroke: "2px #00897b",
          textDecoration: "underline",
          textShadow: "3px 3px rgb(0 0 0 / 40%)",
        }}
      >
        Shadowed
      </Text>
      <Text style={{ ...row, fontSize: 20, WebkitTextFillColor: "#fff" }}>
        <Span style={{ WebkitTextStroke: "thin" }}>thin </Span>
        <Span style={{ WebkitTextStroke: "medium" }}>medium </Span>
        <Span style={{ WebkitTextStroke: "thick" }}>thick</Span>
      </Text>
    </View>
  );
}
