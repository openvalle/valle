export const composition = { width: 320, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 320, height: 160, backgroundColor: "white" }}>
      <Text
        key="text"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 260,
          height: 110,
          backgroundColor: "#f4f7fa",
          padding: 4,
          fontFamily: "sans-serif",
          fontSize: 24,
          lineHeight: "32px",
          color: "#17324d",
          whiteSpace: "pre",
          letterSpacing: 2,
          wordSpacing: 4,
          textIndent: 20,
        }}
      >
        {"MM MM\nMM"}
      </Text>
    </View>
  );
}
