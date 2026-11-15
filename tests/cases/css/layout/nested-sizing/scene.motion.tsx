export const composition = { width: 1010, height: 224, fps: 1, duration: 1 };
const CASES = ["flex", "grid"].flatMap((outer) =>
  ["flex", "grid"].flatMap((inner) =>
    [false, true].map((intrinsic) => ({ outer, inner, intrinsic })),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 1010, height: 224, background: "white" }}>
      {CASES.map((c, i) => (
        <View
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 4) * 250,
            top: 12 + Math.floor(i / 4) * 106,
            width: c.intrinsic ? "max-content" : 220,
            height: 90,
            padding: 4,
            border: "2px solid #17365d",
            display: c.outer,
            alignItems: "start",
            background: "#eef3f8",
            gridTemplateColumns: c.intrinsic ? "max-content" : "1fr",
          }}
        >
          <View
            key="inner"
            style={{
              width: c.intrinsic ? "max-content" : "50%",
              minWidth: 146,
              maxWidth: 160,
              height: 56,
              padding: 6,
              border: "2px solid #17365d",
              display: c.inner,
              gap: 10,
              background: "#dbe9f5",
              gridTemplateColumns: "50px 70px",
              alignItems: "start",
            }}
          >
            <View key="a" style={{ width: 50, height: 40, flexShrink: 0, background: "#2463a0" }} />
            <View key="b" style={{ width: 70, height: 40, flexShrink: 0, background: "#3b856c" }} />
          </View>
        </View>
      ))}
    </View>
  );
}
