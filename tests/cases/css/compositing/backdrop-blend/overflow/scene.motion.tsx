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
          width: 120,
          height: 120,
          backgroundColor: "transparent",
          overflow: "visible",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 140,
            height: 140,
            backgroundColor: "rgb(255 128 0 / 50%)",
            backdropFilter: "invert(1)",
            mixBlendMode: "multiply",
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 30,
              top: 30,
              width: 80,
              height: 70,
              backgroundColor: "#00ff80",
            }}
          />
        </View>
      </View>
      <View
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 120,
          height: 120,
          backgroundColor: "transparent",
          overflow: "hidden",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 140,
            height: 140,
            backgroundColor: "rgb(255 128 0 / 50%)",
            backdropFilter: "invert(1)",
            mixBlendMode: "multiply",
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 30,
              top: 30,
              width: 80,
              height: 70,
              backgroundColor: "#00ff80",
            }}
          />
        </View>
      </View>
    </View>
  );
}
