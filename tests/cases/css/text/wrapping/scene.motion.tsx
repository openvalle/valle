export const composition = { width: 180, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 180, height: 180, backgroundColor: "white" }}>
      <Text
        key="text"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 100,
          height: 120,
          backgroundColor: "#f4f7fa",
          fontFamily: "sans-serif",
          fontSize: 24,
          lineHeight: "32px",
          color: "#17324d",
          wordBreak: "break-all",
        }}
      >
        {"MMMMMMMMMMMM"}
      </Text>
    </View>
  );
}
