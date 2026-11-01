export const composition = { width: 256, height: 320, fps: 1, duration: 1 };

// Spans: colors and weights in a line; a big span on a line of normal text, and small text on a
// line of a big paragraph; nested spans sized by em; a span broken across lines; letter spacing
// and a taller line height in a span; white space collapsing across span edges; centered lines;
// and a hidden span, which keeps its room.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={row}>
        Plain <Span style={{ color: "#e53935", fontWeight: 700 }}>bold red</Span> and{" "}
        <Span style={{ color: "#1e88e5" }}>blue</Span>
      </Text>
      <Text style={row}>
        Small <Span style={{ fontSize: 28 }}>Big</Span> small
      </Text>
      <Text style={{ ...row, fontSize: 30, lineHeight: 1.1 }}>
        <Span style={{ fontSize: 12 }}>only small text</Span>
      </Text>
      <Text style={row}>
        One{" "}
        <Span style={{ fontSize: "1.25em", color: "#8e24aa" }}>
          two <Span style={{ fontSize: "1.25em", fontWeight: 300 }}>three</Span> two
        </Span>{" "}
        one
      </Text>
      <Text style={{ ...row, width: 150 }}>
        Before <Span style={{ color: "#00897b" }}>a span that breaks across the lines</Span> after
      </Text>
      <Text style={{ ...row, lineHeight: 1 }}>
        Tight <Span style={{ letterSpacing: 3, lineHeight: 2 }}>spaced tall</Span> tight
      </Text>
      <Text style={row}>
        {"a  "}
        <Span style={{ color: "#e53935" }}>{"  b  "}</Span>
        {"  c"}
      </Text>
      <Text style={{ ...row, textAlign: "center" }}>
        center <Span style={{ fontSize: 20 }}>mixed</Span> line
      </Text>
      <Text style={row}>
        shown <Span style={{ visibility: "hidden" }}>hidden</Span> shown
      </Text>
    </View>
  );
}
