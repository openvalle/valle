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
          backgroundColor: "#ffff00",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "transparent",
          isolation: "auto",
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
            mixBlendMode: "multiply",
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
          isolation: "isolate",
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
            mixBlendMode: "multiply",
          }}
        />
      </View>
    </View>
  );
}
