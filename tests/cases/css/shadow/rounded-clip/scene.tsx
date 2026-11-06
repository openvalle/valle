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
          backgroundColor: "#eef2f7",
          borderRadius: 24,
          overflow: "hidden",
          boxShadow: "6px 6px 0 #17324d",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: -20,
            top: 30,
            width: 180,
            height: 80,
            backgroundColor: "#176bba",
            transform: "translateY(10px)",
            opacity: 1,
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
          backgroundColor: "#eef2f7",
          borderRadius: 24,
          overflow: "hidden",
          boxShadow: "6px 6px 0 #17324d",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: -20,
            top: 30,
            width: 180,
            height: 80,
            backgroundColor: "#176bba",
            transform: "translateY(10px)",
            opacity: 0.5,
          }}
        />
      </View>
    </View>
  );
}
