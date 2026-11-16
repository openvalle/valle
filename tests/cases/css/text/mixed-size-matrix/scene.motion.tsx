export const composition = { width: 596, height: 402, fps: 1, duration: 1 };
const CASES = ["horizontal-tb", "vertical-rl", "vertical-lr"].flatMap((writing) =>
  ["mixed", "sideways"].flatMap((orientation) =>
    [16, 24].map((size) => ({ writing, orientation, size })),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 596, height: 402, background: "white" }}>
      {CASES.map((c, i) => (
        <Text
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 4) * 146,
            top: 12 + Math.floor(i / 4) * 130,
            width: 132,
            height: 116,
            fontSize: 24,
            lineHeight: "36px",
            color: "#17365d",
            writingMode: c.writing,
            textOrientation: c.orientation,
            background: "#eef3f8",
          }}
        >
          Ag<Span style={{ fontSize: c.size, color: "#9d3c16" }}>pq</Span>
        </Text>
      ))}
    </View>
  );
}
