export const composition = { width: 440, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 440, height: 160, backgroundColor: "white" }}>
      <View
        key="auto"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 120,
          padding: 8,
          border: "2px solid #102a43",
          backgroundColor: "#eef2f7",
        }}
      >
        <View style={{ height: 20, backgroundColor: "#176bba" }} />
        <View style={{ height: 30, backgroundColor: "#16a085" }} />
      </View>
      <View
        key="min"
        style={{
          position: "absolute",
          left: 160,
          top: 20,
          width: 120,
          minHeight: 100,
          padding: 8,
          border: "2px solid #102a43",
          backgroundColor: "#eef2f7",
        }}
      >
        <View style={{ height: 20, backgroundColor: "#176bba" }} />
        <View style={{ height: 30, backgroundColor: "#16a085" }} />
      </View>
      <View
        key="max"
        style={{
          position: "absolute",
          left: 300,
          top: 20,
          width: 120,
          maxHeight: 48,
          overflow: "hidden",
          padding: 8,
          border: "2px solid #102a43",
          backgroundColor: "#eef2f7",
        }}
      >
        <View style={{ height: 20, backgroundColor: "#176bba" }} />
        <View style={{ height: 30, backgroundColor: "#16a085" }} />
      </View>
    </View>
  );
}
