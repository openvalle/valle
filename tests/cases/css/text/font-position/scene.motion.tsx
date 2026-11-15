export const composition = { width: 520, height: 280, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 520, height: 280, padding: 12, backgroundColor: "white" }}>
      <Text key="row0" style={{ fontSize: 48, lineHeight: "64px", color: "#17365d" }}>
        A<Span style={{ fontVariantPosition: "super" }}>123</Span> B
        <Span style={{ fontVariantPosition: "sub" }}>123</Span>
      </Text>
      <Text key="row1" style={{ fontSize: 48, lineHeight: "64px", color: "#17365d" }}>
        A<Span style={{ fontVariantPosition: "super" }}>12Q?</Span> B
        <Span style={{ fontVariantPosition: "sub" }}>12Q?</Span>
      </Text>
      <Text
        key="row2"
        style={{ fontSize: 30, fontSizeAdjust: 1.072, lineHeight: "64px", color: "#17365d" }}
      >
        A<Span style={{ fontVariantPosition: "super" }}>Q?</Span> B
        <Span style={{ fontVariantPosition: "sub" }}>Q?</Span>
      </Text>
      <Text key="row3" style={{ fontSize: 48, lineHeight: "64px", color: "#17365d" }}>
        A<Span style={{ fontVariantPosition: "super", fontVariantCaps: "small-caps" }}>Q?</Span>
      </Text>
    </View>
  );
}
