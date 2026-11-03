export const composition = { width: 256, height: 160, fps: 1, duration: 1 };

// Spans that start with the space a line breaks at, which start on the next line; a space before
// a span instead; and a span that ends with the space.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4 };
  const red = { color: "#e53935" };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={{ ...row, width: 60 }}>
        aaaa<Span style={red}>{" bbbb cc"}</Span>
      </Text>
      <Text style={{ ...row, width: 80 }}>
        aaa aaaa<Span style={red}>{" bbbb"}</Span>
      </Text>
      <Text style={{ ...row, width: 100 }}>
        aaa aaaa aaa<Span style={red}>{" bb"}</Span> cc
      </Text>
      <Text style={{ ...row, width: 60 }}>
        aaaa <Span style={red}>bbbb</Span>
      </Text>
      <Text style={{ ...row, width: 60 }}>
        <Span style={red}>{"aaaa "}</Span>bbbb
      </Text>
    </View>
  );
}
