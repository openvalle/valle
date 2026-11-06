export const composition = { width: 340, height: 210, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 210, backgroundColor: "white" }}>
      <View
        key="grid"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 290,
          height: 160,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "90px 190px",
          gridTemplateRows: "40px 70px 30px",
          gridTemplateAreas: '"head head" "side main" "foot foot"',
          gap: 10,
        }}
      >
        <View key="head" style={{ gridArea: "head", backgroundColor: "#176bba" }} />
        <View key="side" style={{ gridArea: "side", backgroundColor: "#16a085" }} />
        <View key="main" style={{ gridArea: "main", backgroundColor: "#ffb74d" }} />
        <View key="foot" style={{ gridArea: "foot", backgroundColor: "#ad1457" }} />
      </View>
    </View>
  );
}
