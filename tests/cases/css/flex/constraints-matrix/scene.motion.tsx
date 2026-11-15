export const composition = { width: 1080, height: 360, fps: 1, duration: 1 };
const CASES = [172, 212, 252].flatMap((width) =>
  [0, 8].flatMap((gap) =>
    ["border-box", "content-box"].flatMap((boxSizing) =>
      [false, true].map((reverse) => ({ width, gap, boxSizing, reverse })),
    ),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 1080, height: 360, background: "white" }}>
      {CASES.map((c, i) => (
        <View
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 4) * 268,
            top: 12 + Math.floor(i / 4) * 58,
            width: c.width,
            height: 44,
            padding: 4,
            border: "2px solid #17365d",
            background: "#eef3f8",
            display: "flex",
            flexDirection: c.reverse ? "row-reverse" : "row",
            gap: c.gap,
          }}
        >
          <View
            key="a"
            style={{
              width: "50%",
              flexGrow: 1,
              minWidth: 80,
              maxWidth: 100,
              boxSizing: c.boxSizing,
              padding: 2,
              border: "2px solid #17365d",
              background: "#2463a0",
            }}
          />
          <View
            key="b"
            style={{
              width: "50%",
              flexGrow: 1,
              minWidth: 0,
              boxSizing: c.boxSizing,
              padding: 2,
              border: "2px solid #17365d",
              background: "#3b856c",
            }}
          />
        </View>
      ))}
    </View>
  );
}
