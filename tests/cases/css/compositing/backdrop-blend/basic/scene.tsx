export const composition = { width: 660, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 660, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          width: 660,
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
          backgroundColor: "rgb(255 128 0 / 50%)",
          backdropFilter: "invert(1)",
          mixBlendMode: "multiply",
          opacity: 1,
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 60,
            height: 60,
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
          backgroundColor: "rgb(255 128 0 / 50%)",
          backdropFilter: "invert(1)",
          mixBlendMode: "multiply",
          opacity: 0.5,
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 60,
            height: 60,
            backgroundColor: "#00ff80",
          }}
        />
      </View>
      <View
        style={{
          position: "absolute",
          left: 340,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "rgb(255 128 0 / 50%)",
          backdropFilter: "invert(1)",
          mixBlendMode: "multiply",
          opacity: 0,
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 60,
            height: 60,
            backgroundColor: "#00ff80",
          }}
        />
      </View>
      <View
        style={{
          position: "absolute",
          left: 500,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "transparent",
          backdropFilter: "invert(1)",
          mixBlendMode: "multiply",
        }}
      />
    </View>
  );
}
