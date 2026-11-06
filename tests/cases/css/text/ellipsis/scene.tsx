export const composition = { width: 380, height: 110, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 380, height: 110, backgroundColor: "white" }}>
      <Text
        key="clip"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 160,
          height: 60,
          backgroundColor: "#f4f7fa",
          padding: 4,
          fontFamily: "sans-serif",
          fontSize: 24,
          lineHeight: "32px",
          color: "#17324d",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "clip",
        }}
      >
        {"Valle renders every frame"}
      </Text>
      <Text
        key="ellipsis"
        style={{
          position: "absolute",
          left: 200,
          top: 20,
          width: 160,
          height: 60,
          backgroundColor: "#f4f7fa",
          padding: 4,
          fontFamily: "sans-serif",
          fontSize: 24,
          lineHeight: "32px",
          color: "#17324d",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        {"Valle renders every frame"}
      </Text>
    </View>
  );
}
