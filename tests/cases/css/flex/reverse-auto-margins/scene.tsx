export const composition = { width: 420, height: 240, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 420, height: 240, backgroundColor: "white" }}>
      <View
        key="reverse"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 300,
          height: 60,
          backgroundColor: "#eef2f7",
          display: "flex",
          flexDirection: "row-reverse",
          gap: 10,
        }}
      >
        <View key="0" style={{ width: 40, backgroundColor: "#176bba" }} />
        <View key="1" style={{ width: 60, backgroundColor: "#16a085" }} />
        <View key="2" style={{ width: 80, backgroundColor: "#ffb74d" }} />
      </View>
      <View
        key="margin"
        style={{
          position: "absolute",
          left: 20,
          top: 110,
          width: 300,
          height: 60,
          backgroundColor: "#eef2f7",
          display: "flex",
        }}
      >
        <View key="first" style={{ width: 40, backgroundColor: "#176bba" }} />
        <View key="last" style={{ width: 60, marginLeft: "auto", backgroundColor: "#ffb74d" }} />
      </View>
      <View
        key="column"
        style={{
          position: "absolute",
          left: 350,
          top: 20,
          width: 50,
          height: 180,
          backgroundColor: "#eef2f7",
          display: "flex",
          flexDirection: "column-reverse",
          gap: 10,
        }}
      >
        <View key="0" style={{ height: 20, backgroundColor: "#176bba" }} />
        <View key="1" style={{ height: 30, backgroundColor: "#16a085" }} />
        <View key="2" style={{ height: 40, backgroundColor: "#ffb74d" }} />
      </View>
    </View>
  );
}
