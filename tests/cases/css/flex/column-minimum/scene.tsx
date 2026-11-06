export const composition = { width: 380, height: 240, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 380, height: 240, backgroundColor: "white" }}>
      <View
        key="col0"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 160,
          height: 200,
          backgroundColor: "#eef2f7",
          display: "flex",
          flexDirection: "column",
          padding: 10,
          gap: 10,
        }}
      >
        <View key="main" style={{ flex: "1 1 160px", backgroundColor: "#16a085" }}>
          <View style={{ height: 120, backgroundColor: "#176bba" }} />
        </View>
        <View key="foot" style={{ height: 80, flexShrink: 0, backgroundColor: "#ffb74d" }} />
      </View>
      <View
        key="col1"
        style={{
          position: "absolute",
          left: 200,
          top: 20,
          width: 160,
          height: 200,
          backgroundColor: "#eef2f7",
          display: "flex",
          flexDirection: "column",
          padding: 10,
          gap: 10,
        }}
      >
        <View key="main" style={{ flex: "1 1 160px", backgroundColor: "#16a085", minHeight: 0 }}>
          <View style={{ height: 120, backgroundColor: "#176bba" }} />
        </View>
        <View key="foot" style={{ height: 80, flexShrink: 0, backgroundColor: "#ffb74d" }} />
      </View>
    </View>
  );
}
