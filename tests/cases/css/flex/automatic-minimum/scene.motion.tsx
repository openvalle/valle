export const composition = { width: 360, height: 190, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 190, backgroundColor: "white" }}>
      <View
        key="auto"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 220,
          height: 60,
          backgroundColor: "#eef2f7",
          display: "flex",
        }}
      >
        <View key="item" style={{ flex: "1 1 200px", backgroundColor: "#16a085" }}>
          <View style={{ width: 180, height: 20, backgroundColor: "#176bba" }} />
        </View>
        <View key="fixed" style={{ width: 80, flexShrink: 0, backgroundColor: "#ffb74d" }} />
      </View>
      <View
        key="zero"
        style={{
          position: "absolute",
          left: 20,
          top: 110,
          width: 220,
          height: 60,
          backgroundColor: "#eef2f7",
          display: "flex",
        }}
      >
        <View key="item" style={{ flex: "1 1 200px", backgroundColor: "#16a085", minWidth: 0 }}>
          <View style={{ width: 180, height: 20, backgroundColor: "#176bba" }} />
        </View>
        <View key="fixed" style={{ width: 80, flexShrink: 0, backgroundColor: "#ffb74d" }} />
      </View>
    </View>
  );
}
