export const composition = { width: 380, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 380, height: 180, backgroundColor: "white" }}>
      <View
        key="min"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: "min-content",
          display: "flex",
          flexWrap: "wrap",
          gap: 10,
          backgroundColor: "#eef2f7",
        }}
      >
        <View style={{ width: 60, height: 30, flexShrink: 0, backgroundColor: "#176bba" }} />
        <View style={{ width: 100, height: 30, flexShrink: 0, backgroundColor: "#16a085" }} />
      </View>
      <View
        key="max"
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: "max-content",
          display: "flex",
          flexWrap: "wrap",
          gap: 10,
          backgroundColor: "#eef2f7",
        }}
      >
        <View style={{ width: 60, height: 30, flexShrink: 0, backgroundColor: "#176bba" }} />
        <View style={{ width: 100, height: 30, flexShrink: 0, backgroundColor: "#16a085" }} />
      </View>
    </View>
  );
}
