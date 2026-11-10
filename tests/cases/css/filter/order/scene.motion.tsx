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
          filter: "invert(1) brightness(0.5)",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 140,
            height: 140,
            backgroundColor: "#4080c0",
          }}
        />
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 60,
            height: 60,
            backgroundColor: "#ff0000",
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
          filter: "brightness(0.5) invert(1)",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 140,
            height: 140,
            backgroundColor: "#4080c0",
          }}
        />
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 60,
            height: 60,
            backgroundColor: "#ff0000",
          }}
        />
      </View>
    </View>
  );
}
