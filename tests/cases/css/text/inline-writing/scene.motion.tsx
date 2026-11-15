export const composition = { width: 520, height: 300, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View
      style={{
        width: 520,
        height: 300,
        background: "white",
        fontSize: 20,
        lineHeight: "32px",
        color: "#17365d",
      }}
    >
      {["vertical-rl", "vertical-lr"].map((writingMode, i) => (
        <Text
          key={`column${i}`}
          style={{
            position: "absolute",
            left: 12 + i * 160,
            top: 12,
            width: 120,
            height: 180,
            writingMode,
          }}
        >
          AB{" "}
          <View
            style={{
              display: "inline-block",
              width: 30,
              height: 40,
              writingMode: "horizontal-tb",
              background: "#ffe7ad",
              verticalAlign: "middle",
            }}
          >
            <Text style={{ fontSize: 14, lineHeight: "20px" }}>OK</Text>
          </View>{" "}
          CD EF GH
        </Text>
      ))}
      <Text
        key="rtl"
        style={{
          position: "absolute",
          left: 12,
          top: 212,
          width: 496,
          height: 76,
          direction: "rtl",
        }}
      >
        AB{" "}
        <Span style={{ background: "#ffe7ad", padding: "0 4px", verticalAlign: "super" }}>
          <View
            style={{
              display: "inline-block",
              width: 30,
              height: 28,
              background: "#27ae80",
              verticalAlign: "middle",
            }}
          />
        </Span>{" "}
        CD{" "}
        <View
          style={{
            display: "inline-grid",
            gridTemplateColumns: "16px 16px",
            gap: 3,
            padding: 3,
            background: "#d6eaf8",
            verticalAlign: "bottom",
          }}
        >
          <View style={{ height: 16, background: "#2b71b8" }} />
          <View style={{ height: 16, background: "#27ae80" }} />
        </View>{" "}
        EF
      </Text>
    </View>
  );
}
