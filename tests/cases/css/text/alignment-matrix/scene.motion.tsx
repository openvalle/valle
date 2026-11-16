export const composition = { width: 624, height: 364, fps: 1, duration: 1 };
const CASES = ["inline-block", "inline-flex", "inline-grid"].flatMap((display) =>
  ["baseline", "middle", "top", "bottom"].map((align) => ({ display, align })),
);
export default function Scene() {
  return (
    <View style={{ width: 624, height: 364, background: "white" }}>
      {CASES.map((c, i) => (
        <Text
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 3) * 204,
            top: 12 + Math.floor(i / 3) * 88,
            width: 192,
            height: 72,
            fontSize: 24,
            lineHeight: "36px",
            color: "#17365d",
            background: "#eef3f8",
          }}
        >
          A
          <Span
            style={{
              fontSize: 14,
              lineHeight: "20px",
              verticalAlign: "super",
              background: "#d6eaf8",
            }}
          >
            xy
          </Span>
          <View
            key="badge"
            style={{
              display: c.display,
              width: 40,
              height: 24,
              padding: 2,
              border: "2px solid #b58300",
              background: "#ffebae",
              verticalAlign: c.align,
            }}
          />
          B
        </Text>
      ))}
    </View>
  );
}
