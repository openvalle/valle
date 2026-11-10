export const composition = { width: 360, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 180, backgroundColor: "white" }}>
      <View
        key="grow"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 300,
          height: 50,
          backgroundColor: "#eef2f7",
          display: "flex",
          gap: 12,
        }}
      >
        <View key="one" style={{ flex: "1 1 0px", backgroundColor: "#176bba" }} />
        <View key="two" style={{ flex: "2 1 0px", backgroundColor: "#16a085" }} />
      </View>
      <View
        key="shrink"
        style={{
          position: "absolute",
          left: 20,
          top: 100,
          width: 240,
          height: 50,
          backgroundColor: "#eef2f7",
          display: "flex",
        }}
      >
        <View key="large" style={{ width: 200, flexShrink: 1, backgroundColor: "#176bba" }} />
        <View key="small" style={{ width: 100, flexShrink: 1, backgroundColor: "#ffb74d" }} />
      </View>
    </View>
  );
}
