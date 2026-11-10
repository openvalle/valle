export const composition = { width: 180, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 180, height: 160, backgroundColor: "white" }}>
      <Text
        key="text"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 108,
          height: 108,
          backgroundColor: "#f4f7fa",
          padding: 4,
          fontFamily: "sans-serif",
          fontSize: 24,
          lineHeight: "32px",
          color: "#17324d",
          overflowWrap: "anywhere",
        }}
      >
        {"MMMMMMMM"}
      </Text>
    </View>
  );
}
