export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "transparent",
          overflow: "visible",
          border: "2px solid #17324d",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 10,
            top: 30,
            width: 120,
            height: 80,
            backgroundColor: "#176bba",
            filter: "blur(6px)",
          }}
        />
      </View>
      <View
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "transparent",
          overflow: "hidden",
          border: "2px solid #17324d",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 10,
            top: 30,
            width: 120,
            height: 80,
            backgroundColor: "#176bba",
            filter: "blur(6px)",
          }}
        />
      </View>
    </View>
  );
}
