export const composition = { width: 220, height: 140, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 220, height: 140, backgroundColor: "white" }}>
      <Text
        key="text"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 160,
          height: 100,
          backgroundColor: "#f4f7fa",
          fontFamily: "sans-serif",
          fontSize: 24,
          lineHeight: "32px",
          color: "#17324d",
          whiteSpace: "normal",
        }}
      >
        {"Alpha beta gamma delta"}
      </Text>
    </View>
  );
}
