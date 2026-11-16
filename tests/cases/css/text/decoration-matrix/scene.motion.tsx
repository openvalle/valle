export const composition = { width: 888, height: 662, fps: 1, duration: 1 };
const CASES = ["solid", "double", "dotted", "dashed", "wavy"].flatMap((decoration) =>
  ["horizontal-tb", "vertical-rl", "vertical-lr"].flatMap((writing) =>
    [0.5, 1].map((opacity) => ({ decoration, writing, opacity })),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 888, height: 662, background: "white" }}>
      {CASES.map((c, i) => (
        <Text
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 6) * 146,
            top: 12 + Math.floor(i / 6) * 130,
            width: 132,
            height: 116,
            fontSize: 24,
            lineHeight: "36px",
            color: "#17365d",
            writingMode: c.writing,
            textDecorationLine: "underline",
            textDecorationStyle: c.decoration,
            textDecorationThickness: "2px",
            textUnderlineOffset: "3px",
            textDecorationSkipInk: "none",
            textUnderlinePosition: "under left",
            WebkitTextStroke: "1px #17365d",
            textShadow: "2px 2px 0px #b6c7d8",
            opacity: c.opacity,
          }}
        >
          Ag<Span style={{ fontSize: 24, color: "#9d3c16", WebkitTextStrokeWidth: 0 }}>pq</Span>
        </Text>
      ))}
    </View>
  );
}
