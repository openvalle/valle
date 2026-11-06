export const composition = { width: 360, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 160, backgroundColor: "white" }}>
      <Text
        key="text"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 320,
          height: 110,
          backgroundColor: "#f4f7fa",
          fontFamily: "sans-serif",
          fontSize: 20,
          lineHeight: "48px",
          color: "#17324d",
        }}
      >
        <Span>{"Small "}</Span>
        <Span style={{ fontSize: 36, fontWeight: 700 }}>{"BIG"}</Span>
        <Span style={{ fontStyle: "italic" }}>{" italic"}</Span>
      </Text>
    </View>
  );
}
