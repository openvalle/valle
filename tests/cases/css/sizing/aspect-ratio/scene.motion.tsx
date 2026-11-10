export const composition = { width: 340, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 160, backgroundColor: "white" }}>
      <View
        key="width"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 160,
          aspectRatio: "2 / 1",
          backgroundColor: "#176bba",
        }}
      />
      <View
        key="height"
        style={{
          position: "absolute",
          left: 220,
          top: 20,
          height: 90,
          aspectRatio: "1 / 1",
          backgroundColor: "#16a085",
        }}
      />
    </View>
  );
}
