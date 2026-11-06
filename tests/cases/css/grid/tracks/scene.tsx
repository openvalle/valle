export const composition = { width: 380, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 380, height: 180, backgroundColor: "white" }}>
      <View
        key="grid"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 340,
          height: 140,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "80px 1fr 2fr",
          gridTemplateRows: "40px 60px",
          gap: 10,
        }}
      >
        <View key="0" style={{ backgroundColor: "#176bba" }} />
        <View key="1" style={{ backgroundColor: "#16a085" }} />
        <View key="2" style={{ backgroundColor: "#ffb74d" }} />
        <View key="3" style={{ backgroundColor: "#ad1457" }} />
        <View key="4" style={{ backgroundColor: "#176bba" }} />
        <View key="5" style={{ backgroundColor: "#16a085" }} />
      </View>
    </View>
  );
}
