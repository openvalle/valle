export const composition = { width: 500, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 500, height: 160, backgroundColor: "white" }}>
      <View
        key="sparse"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 200,
          height: 120,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "repeat(3, 60px)",
          gridAutoRows: "40px",
          gridAutoFlow: "row",
          gap: 10,
        }}
      >
        <View key="first" style={{ gridColumn: "span 2", backgroundColor: "#176bba" }} />
        <View key="second" style={{ gridColumn: "span 2", backgroundColor: "#16a085" }} />
        <View key="small" style={{ backgroundColor: "#ffb74d" }} />
      </View>
      <View
        key="dense"
        style={{
          position: "absolute",
          left: 270,
          top: 20,
          width: 200,
          height: 120,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "repeat(3, 60px)",
          gridAutoRows: "40px",
          gridAutoFlow: "row dense",
          gap: 10,
        }}
      >
        <View key="first" style={{ gridColumn: "span 2", backgroundColor: "#176bba" }} />
        <View key="second" style={{ gridColumn: "span 2", backgroundColor: "#16a085" }} />
        <View key="small" style={{ backgroundColor: "#ffb74d" }} />
      </View>
    </View>
  );
}
