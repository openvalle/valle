export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          width: 340,
          height: 180,
          backgroundColor: "#4080c0",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "rgb(255 255 255 / 25%)",
          backdropFilter: "invert(1)",
          clipPath: "inset(10px)",
          maskImage: "linear-gradient(to right, black 50%, transparent 50%)",
          opacity: 1,
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 30,
            top: 30,
            width: 80,
            height: 80,
            backgroundColor: "#00ff80",
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
          backgroundColor: "rgb(255 255 255 / 25%)",
          backdropFilter: "invert(1)",
          clipPath: "inset(10px)",
          maskImage: "linear-gradient(to right, black 50%, transparent 50%)",
          opacity: 0.5,
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 30,
            top: 30,
            width: 80,
            height: 80,
            backgroundColor: "#00ff80",
          }}
        />
      </View>
    </View>
  );
}
