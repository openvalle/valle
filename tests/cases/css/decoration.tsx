export const composition = { width: 256, height: 320, fps: 1, duration: 1 };

// Text decorations: underlines that skip the ink of descenders, and one that does not; an
// overline and a line-through; a thick one further down; a View's on the text in flow in it, a
// flex item's too, but not on an absolute box; a Text's through a big Span and the Span's own,
// each in its own font; a Span's over a Text's on broken lines; and a hidden Span, which keeps
// its room but shows no decoration.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={{ ...row, fontSize: 20, textDecoration: "underline" }}>Hamburger gyp</Text>
      <Text
        style={{ ...row, fontSize: 20, textDecoration: "underline", textDecorationSkipInk: "none" }}
      >
        Hamburger gyp
      </Text>
      <Text style={{ ...row, fontSize: 14, textDecoration: "overline line-through #e53935" }}>
        Struck through and over
      </Text>
      <Text style={{ ...row, textDecoration: "underline 3px #1e88e5", textUnderlineOffset: 4 }}>
        Thick and lower, jumpy
      </Text>
      <View style={{ position: "relative", fontSize: 12, textDecoration: "underline #00897b" }}>
        <View style={{ display: "flex" }}>
          <Text style={row}>In a flex row</Text>
        </View>
        <Text style={{ position: "absolute", right: 0, top: 0 }}>absolute</Text>
      </View>
      <Text style={{ ...row, textDecoration: "line-through" }}>
        Small{" "}
        <Span style={{ fontSize: 26, color: "#8e24aa", textDecoration: "underline 10% #8e24aa" }}>
          Big
        </Span>{" "}
        small
      </Text>
      <Text style={{ ...row, width: 150, textDecoration: "underline" }}>
        Before{" "}
        <Span style={{ color: "#00897b", textDecorationLine: "overline" }}>a span that breaks</Span>{" "}
        after
      </Text>
      <Text style={{ ...row, textDecoration: "underline" }}>
        shown <Span style={{ visibility: "hidden" }}>hidden</Span> shown
      </Text>
    </View>
  );
}
