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
          backgroundColor: "#176bba",
          clipPath: "inset(20px 10px 30px 20px)",
          maskImage: "linear-gradient(to right, black 50%, transparent 50%)",
          opacity: 1,
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 20,
            top: 40,
            width: 120,
            height: 80,
            backgroundColor: "#ffb74d",
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
          backgroundColor: "#176bba",
          clipPath: "inset(20px 10px 30px 20px)",
          maskImage: "linear-gradient(to right, black 50%, transparent 50%)",
          opacity: 0.5,
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 20,
            top: 40,
            width: 120,
            height: 80,
            backgroundColor: "#ffb74d",
          }}
        />
      </View>
    </View>
  );
}
