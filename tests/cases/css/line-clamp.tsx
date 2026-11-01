export const composition = { width: 256, height: 288, fps: 1, duration: 1 };

// Clamped lines: text cut short on its second line; lines with room for the ellipsis after them,
// start and centered; text of exactly the lines it shows; an underlined Span that runs into hidden lines;
// -webkit-box without a vertical orient, and an orient without -webkit-box, neither clamped; a
// set height, which clamping keeps; and a clamp that does not clip, which paints the lines past
// it.
export default function Scene() {
  const row = { background: "#eceff1", width: 120, fontSize: 14, overflow: "hidden" };
  const clamp = (n: number) => ({
    ...row,
    display: "-webkit-box",
    WebkitBoxOrient: "vertical",
    WebkitLineClamp: n,
  });
  const fox = "The quick brown fox jumps over the lazy dog";
  const broken = "One\nTwo\nThree";
  return (
    <View
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "flex-start",
        gap: "4px 8px",
        padding: 4,
        color: "#263238",
      }}
    >
      <Text style={clamp(2)}>{fox}</Text>
      <Text style={{ ...clamp(2), whiteSpace: "pre-line" }}>{broken}</Text>
      <Text style={{ ...clamp(2), whiteSpace: "pre-line", textAlign: "center" }}>{broken}</Text>
      <Text style={{ ...clamp(2), whiteSpace: "pre-line" }}>{"One\nTwo"}</Text>
      <Text style={clamp(2)}>
        The quick{" "}
        <Span style={{ color: "#1e88e5", textDecoration: "underline" }}>brown fox jumps over</Span>{" "}
        the lazy dog
      </Text>
      <Text style={{ ...clamp(2), WebkitBoxOrient: "horizontal" }}>{fox}</Text>
      <Text style={{ ...row, WebkitBoxOrient: "vertical", WebkitLineClamp: 2 }}>{fox}</Text>
      <Text style={{ ...clamp(1), height: 60 }}>{fox}</Text>
      <Text style={{ ...clamp(2), overflow: "visible" }}>{fox}</Text>
    </View>
  );
}
