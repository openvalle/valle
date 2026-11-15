export const composition = { width: 604, height: 268, fps: 1, duration: 1 };
export const controls = {
  resources: { CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }) },
};
export default function Scene() {
  return (
    <View style={{ position: "relative", width: 604, height: 268, backgroundColor: "white" }}>
      <View
        key="p0"
        style={{
          position: "absolute",
          left: 12,
          top: 12,
          width: 280,
          height: 244,
          padding: 10,
          border: "2px solid #17365d",
          backgroundColor: "#eef3f8",
          writingMode: "vertical-rl",
          display: "flex",
          flexDirection: "column",
          gap: 8,
          alignItems: "start",
          fontFamily: "CJK",
          fontSize: 20,
          lineHeight: "28px",
        }}
      >
        <Text
          key="vertical"
          style={{
            width: 60,
            height: 200,
            padding: 4,
            backgroundColor: "#c6d6e6",
            color: "#17365d",
            flexShrink: 0,
          }}
        >
          春风
          <Span style={{ padding: "2%", border: "1px solid #da803d", color: "#245b9d" }}>山水</Span>
          长天
        </Text>
        <Text
          key="horizontal"
          style={{
            writingMode: "horizontal-tb",
            width: 100,
            height: 60,
            backgroundColor: "#e6d9c6",
            fontSize: 18,
            lineHeight: "24px",
            flexShrink: 0,
          }}
        >
          横排 ABC
        </Text>
        <svg
          key="svg"
          viewBox="0 0 20 10"
          style={{ height: 30, alignSelf: "center", flexShrink: 0 }}
        >
          <rect width="20" height="10" fill="#3b856c" />
          <circle cx="5" cy="5" r="3" fill="white" />
        </svg>
      </View>
      <View
        key="p1"
        style={{
          position: "absolute",
          left: 312,
          top: 12,
          width: 280,
          height: 244,
          padding: 10,
          border: "2px solid #17365d",
          backgroundColor: "#eef3f8",
          writingMode: "vertical-lr",
          display: "flex",
          flexDirection: "column",
          gap: 8,
          alignItems: "start",
          fontFamily: "CJK",
          fontSize: 20,
          lineHeight: "28px",
        }}
      >
        <Text
          key="vertical"
          style={{
            width: 60,
            height: 200,
            padding: 4,
            backgroundColor: "#c6d6e6",
            color: "#17365d",
            flexShrink: 0,
          }}
        >
          春风
          <Span style={{ padding: "2%", border: "1px solid #da803d", color: "#245b9d" }}>山水</Span>
          长天
        </Text>
        <Text
          key="horizontal"
          style={{
            writingMode: "horizontal-tb",
            width: 100,
            height: 60,
            backgroundColor: "#e6d9c6",
            fontSize: 18,
            lineHeight: "24px",
            flexShrink: 0,
          }}
        >
          横排 ABC
        </Text>
        <svg
          key="svg"
          viewBox="0 0 20 10"
          style={{ height: 30, alignSelf: "center", flexShrink: 0 }}
        >
          <rect width="20" height="10" fill="#3b856c" />
          <circle cx="5" cy="5" r="3" fill="white" />
        </svg>
      </View>
    </View>
  );
}
