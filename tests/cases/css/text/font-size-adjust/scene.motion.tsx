export const composition = { width: 480, height: 220, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 480, height: 220, backgroundColor: "white" }}>
      <Text
        style={{
          position: "absolute",
          left: 12,
          top: 8,
          fontSize: 24,
          fontSizeAdjust: 1.072,
          lineHeight: "64px",
          color: "#17365d",
        }}
      >
        Hello
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 252,
          top: 8,
          fontSize: 48,
          lineHeight: "64px",
          color: "#17365d",
        }}
      >
        Hello
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 12,
          top: 80,
          fontSize: 24,
          fontSizeAdjust: 1.072,
          lineHeight: "64px",
          color: "#17365d",
        }}
      >
        Big<Span style={{ fontSizeAdjust: 0.536 }}> small</Span>
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 252,
          top: 80,
          fontSize: 48,
          lineHeight: "64px",
          color: "#17365d",
        }}
      >
        Big<Span style={{ fontSize: 24 }}> small</Span>
      </Text>
      <View
        key="units"
        style={{
          position: "absolute",
          left: 12,
          top: 164,
          fontSize: 31.25,
          fontSizeAdjust: 1.072,
          width: "1em",
          height: "1ex",
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 252,
          top: 164,
          width: 31.25,
          height: 33.5,
          backgroundColor: "#2878b8",
        }}
      />
    </View>
  );
}
