export const composition = { width: 420, height: 240, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 420, height: 240, backgroundColor: "white" }}>
      <Text
        key="rtl"
        style={{
          position: "absolute",
          left: 12,
          top: 16,
          width: 230,
          fontSize: 20,
          lineHeight: "40px",
          direction: "rtl",
          unicodeBidi: "bidi-override",
          color: "#182433",
        }}
      >
        {"AB "}
        <Span
          style={{ padding: "3px 8px", border: "2px solid #28618e", backgroundColor: "#f5d483" }}
        >
          CD EF GH IJ KL MN OP
        </Span>
        {" QR"}
      </Text>
      <Text
        key="vertical"
        style={{
          position: "absolute",
          left: 290,
          top: 16,
          width: 100,
          height: 160,
          fontSize: 20,
          lineHeight: "40px",
          writingMode: "vertical-rl",
          color: "#182433",
        }}
      >
        {"A "}
        <Span
          style={{
            padding: "8px 3px",
            border: "2px solid #28618e",
            background: "linear-gradient(90deg, #b2e1d3, #f5d483)",
          }}
        >
          BC DE FG HI JK LM
        </Span>
        {" N"}
      </Text>
    </View>
  );
}
