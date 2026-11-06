export const composition = { width: 360, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 30,
          top: 30,
          width: 60,
          height: 40,
          backgroundColor: "#eef2f7",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 30,
          top: 30,
          width: 60,
          height: 40,
          backgroundColor: "#176bba",
          transform: "translate(20px, 40px)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 170,
          top: 30,
          width: 60,
          height: 40,
          backgroundColor: "#16a085",
          transformOrigin: "0px 0px",
          transform: "scale(2, 2)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 280,
          top: 30,
          width: 60,
          height: 40,
          backgroundColor: "#ffb74d",
          transformOrigin: "0px 0px",
          transform: "rotate(90deg)",
        }}
      />
    </View>
  );
}
