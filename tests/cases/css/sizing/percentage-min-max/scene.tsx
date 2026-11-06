export const composition = { width: 360, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 180, backgroundColor: "white" }}>
      <View
        key="parent"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 320,
          height: 140,
          backgroundColor: "#eef2f7",
        }}
      >
        <View
          key="max"
          style={{ width: "75%", maxWidth: 200, height: 30, backgroundColor: "#176bba" }}
        />
        <View
          key="min"
          style={{ width: "25%", minWidth: 120, height: 30, backgroundColor: "#16a085" }}
        />
        <View
          key="calc"
          style={{ width: "calc(50% - 20px)", height: 30, backgroundColor: "#ffb74d" }}
        />
      </View>
    </View>
  );
}
