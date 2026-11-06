export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 30,
          top: 30,
          width: 100,
          height: 100,
          backgroundColor: "#ffb74d",
          boxShadow: "8px 10px 0px 4px #176bba",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 200,
          top: 30,
          width: 100,
          height: 100,
          backgroundColor: "#ffb74d",
          boxShadow: "inset 8px 10px 0px 4px #176bba",
        }}
      />
    </View>
  );
}
