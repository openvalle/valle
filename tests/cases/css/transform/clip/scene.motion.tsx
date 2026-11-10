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
          overflow: "hidden",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 20,
            top: 20,
            width: 140,
            height: 80,
            backgroundColor: "#176bba",
            transform: "rotate(90deg)",
            transformOrigin: "0 0",
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 40,
              top: 10,
              width: 100,
              height: 60,
              backgroundColor: "#ffb74d",
            }}
          />
        </View>
      </View>
      <View
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "#eef2f7",
          overflow: "hidden",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 20,
            top: 20,
            width: 140,
            height: 80,
            backgroundColor: "#176bba",
            transform: "rotate(90deg)",
            transformOrigin: "0 0",
            clipPath: "inset(10px 20px)",
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 40,
              top: 10,
              width: 100,
              height: 60,
              backgroundColor: "#ffb74d",
            }}
          />
        </View>
      </View>
    </View>
  );
}
