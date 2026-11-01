export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// White space and case by Span: kept spaces and breaks in a span of collapsing text, a span that
// does not wrap in text that does, collapsing text in a paragraph that keeps its white space,
// and words spaced and cased in spans.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, width: 180 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={row}>
        {"collapse   "}
        <Span style={{ whiteSpace: "pre", color: "#e53935" }}>{"  kept\n  spaces  "}</Span>
        {"   then collapse"}
      </Text>
      <Text style={row}>
        Words wrap here but{" "}
        <Span style={{ whiteSpace: "nowrap", color: "#1e88e5" }}>these words stay together</Span>{" "}
        always
      </Text>
      <Text style={{ ...row, whiteSpace: "pre-wrap" }}>
        {"kept  spaces "}
        <Span style={{ whiteSpace: "normal", color: "#8e24aa" }}>{"  collapsed   here  "}</Span>
        {" kept  again"}
      </Text>
      <Text style={{ ...row, textTransform: "capitalize" }}>
        every word{" "}
        <Span style={{ textTransform: "uppercase", wordSpacing: 6, color: "#00897b" }}>
          loud and spaced
        </Span>{" "}
        <Span style={{ textTransform: "none" }}>left as is</Span>
      </Text>
    </View>
  );
}
