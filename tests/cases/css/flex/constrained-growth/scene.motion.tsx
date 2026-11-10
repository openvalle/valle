export const composition = { width: 340, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 160, backgroundColor: "white" }}>
      <View
        key="row"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 300,
          height: 100,
          backgroundColor: "#eef2f7",
          display: "flex",
        }}
      >
        <View key="capped" style={{ flex: "1 1 0px", maxWidth: 80, backgroundColor: "#176bba" }} />
        <View key="second" style={{ flex: "1 1 0px", backgroundColor: "#16a085" }} />
        <View key="third" style={{ flex: "1 1 0px", backgroundColor: "#ffb74d" }} />
      </View>
    </View>
  );
}
