export const composition = { width: 420, height: 260, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 420, height: 260, padding: 16, backgroundColor: "white" }}>
      <Text key="wrap" style={{ width: 230, fontSize: 20, lineHeight: "40px", color: "#182433" }}>
        {"One "}
        <Span
          style={{
            padding: "3px 8px",
            border: "2px solid #28618e",
            background: "linear-gradient(90deg, #f5d483, #b2e1d3)",
          }}
        >
          two three four five six seven
        </Span>
        {" end"}
      </Text>
      <Text key="nested" style={{ fontSize: 20, lineHeight: "40px", color: "#182433" }}>
        {"A "}
        <Span
          style={{ padding: "2px 10px", border: "2px solid #28618e", backgroundColor: "#dae9f1" }}
        >
          B
          <Span
            style={{ padding: "0 6px", border: "2px solid #a63a30", backgroundColor: "#f4d37d" }}
          >
            C
          </Span>
          D
        </Span>
        {" E"}
      </Text>
      <Text key="empty" style={{ fontSize: 20, lineHeight: "40px" }}>
        A
        <Span
          style={{ padding: "2px 8px", border: "2px solid #a63a30", backgroundColor: "#f4d37d" }}
        />
        {" B"}
      </Text>
    </View>
  );
}
