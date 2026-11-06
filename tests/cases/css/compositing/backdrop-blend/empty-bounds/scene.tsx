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
          width: 0,
          height: 100,
          backgroundColor: "transparent",
          mixBlendMode: "multiply",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 100,
            height: 100,
            backgroundColor: "#00ff80",
          }}
        />
      </View>
      <View
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 0,
          height: 100,
          backgroundColor: "transparent",
          mixBlendMode: "multiply",
          backdropFilter: "invert(1)",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 100,
            height: 100,
            backgroundColor: "#00ff80",
          }}
        />
      </View>
    </View>
  );
}
