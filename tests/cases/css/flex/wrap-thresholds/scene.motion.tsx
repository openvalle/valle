export const composition = { width: 1060, height: 552, fps: 1, duration: 1 };
const CASES = [-2, 0, 2].flatMap((delta) =>
  [0, 8].flatMap((gap) =>
    ["nowrap", "wrap", "wrap-reverse"].flatMap((wrap) =>
      [false, true].map((reverse) => ({ delta, gap, wrap, reverse })),
    ),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 1060, height: 552, background: "white" }}>
      {CASES.map((c, i) => (
        <View
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 6) * 174,
            top: 12 + Math.floor(i / 6) * 90,
            width: 132 + c.gap + c.delta,
            height: 80,
            padding: 4,
            border: "2px solid #17365d",
            background: "#eef3f8",
            display: "flex",
            flexDirection: c.reverse ? "row-reverse" : "row",
            flexWrap: c.wrap,
            gap: c.gap,
            alignContent: "flex-start",
            alignItems: "flex-start",
          }}
        >
          <View key="a" style={{ width: 60, height: 20, minWidth: 0, background: "#2463a0" }} />
          <View key="b" style={{ width: 60, height: 20, minWidth: 0, background: "#3b856c" }} />
        </View>
      ))}
    </View>
  );
}
