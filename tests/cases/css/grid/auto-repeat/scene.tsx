export const composition = { width: 360, height: 200, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 200, backgroundColor: "white" }}>
      <View
        key="auto-fill"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 320,
          height: 60,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(80px, 1fr))",
          gap: 10,
        }}
      >
        <View key="first" style={{ backgroundColor: "#176bba" }} />
        <View key="second" style={{ backgroundColor: "#16a085" }} />
      </View>
      <View
        key="auto-fit"
        style={{
          position: "absolute",
          left: 20,
          top: 110,
          width: 320,
          height: 60,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(80px, 1fr))",
          gap: 10,
        }}
      >
        <View key="first" style={{ backgroundColor: "#176bba" }} />
        <View key="second" style={{ backgroundColor: "#16a085" }} />
      </View>
    </View>
  );
}
