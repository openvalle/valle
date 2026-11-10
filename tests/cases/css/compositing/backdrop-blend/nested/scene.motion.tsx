export const composition = { width: 500, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 500, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          width: 500,
          height: 180,
          backgroundColor: "#a05020",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "#4080c0",
          opacity: 1,
          isolation: "isolate",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 10,
            top: 10,
            width: 120,
            height: 120,
            backgroundColor: "rgb(255 128 0 / 50%)",
            backdropFilter: "invert(1)",
            mixBlendMode: "multiply",
            opacity: 1,
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 30,
              top: 30,
              width: 60,
              height: 60,
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
          width: 140,
          height: 140,
          backgroundColor: "#4080c0",
          opacity: 0.5,
          isolation: "isolate",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 10,
            top: 10,
            width: 120,
            height: 120,
            backgroundColor: "rgb(255 128 0 / 50%)",
            backdropFilter: "invert(1)",
            mixBlendMode: "multiply",
            opacity: 1,
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 30,
              top: 30,
              width: 60,
              height: 60,
              backgroundColor: "#00ff80",
            }}
          />
        </View>
      </View>
      <View
        style={{
          position: "absolute",
          left: 340,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "#4080c0",
          opacity: 0.5,
          isolation: "isolate",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 10,
            top: 10,
            width: 120,
            height: 120,
            backgroundColor: "rgb(255 128 0 / 50%)",
            backdropFilter: "invert(1)",
            mixBlendMode: "multiply",
            opacity: 0.5,
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 30,
              top: 30,
              width: 60,
              height: 60,
              backgroundColor: "#00ff80",
            }}
          />
        </View>
      </View>
    </View>
  );
}
