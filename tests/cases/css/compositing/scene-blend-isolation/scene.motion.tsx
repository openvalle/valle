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
          backgroundColor: "#ffff00",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 100,
          height: 100,
          backgroundColor: "#00ffff",
          mixBlendMode: "multiply",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "transparent",
          transformStyle: "preserve-3d",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 100,
            height: 100,
            backgroundColor: "#00ffff",
            transform: "translateZ(1px)",
            mixBlendMode: "multiply",
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
          backgroundColor: "#ffff00",
          transformStyle: "preserve-3d",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 100,
            height: 100,
            backgroundColor: "#00ffff",
            transform: "translateZ(1px)",
            mixBlendMode: "multiply",
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
          transformStyle: "preserve-3d",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 100,
            height: 100,
            backgroundColor: "#00ffff",
            transform: "translateZ(1px)",
            mixBlendMode: "multiply",
          }}
        />
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 100,
            height: 100,
            backgroundColor: "#ff00ff",
            transform: "translateZ(2px)",
            mixBlendMode: "multiply",
          }}
        />
      </View>
    </View>
  );
}
