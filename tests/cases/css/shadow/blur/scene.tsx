export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 40,
          top: 40,
          width: 100,
          height: 80,
          backgroundColor: "#ffb74d",
          boxShadow: "4px 6px 8px #176bba",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 200,
          top: 40,
          width: 100,
          height: 80,
          backgroundColor: "#ffb74d",
          boxShadow: "inset 4px 6px 8px #176bba",
        }}
      />
    </View>
  );
}
