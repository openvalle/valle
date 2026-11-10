export const composition = { width: 300, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 300, height: 160, backgroundColor: "white" }}>
      <Text
        key="text"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 240,
          height: 120,
          backgroundColor: "#f4f7fa",
          fontFamily: "sans-serif",
          fontSize: 24,
          lineHeight: "32px",
          whiteSpace: "pre",
          textAlign: "right",
          color: "#17324d",
        }}
      >
        {"MMMM\nMM"}
      </Text>
    </View>
  );
}
