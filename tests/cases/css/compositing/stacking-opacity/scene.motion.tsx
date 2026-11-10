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
          backgroundColor: "#eef2f7",
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
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 110,
            height: 110,
            backgroundColor: "#176bba",
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 60,
              top: 60,
              width: 80,
              height: 80,
              backgroundColor: "#ffb74d",
              zIndex: 100,
            }}
          />
        </View>
        <View
          style={{
            position: "absolute",
            left: 30,
            top: 30,
            width: 80,
            height: 80,
            backgroundColor: "#16a085",
            zIndex: 1,
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
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 110,
            height: 110,
            backgroundColor: "#176bba",
            opacity: 0.5,
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 60,
              top: 60,
              width: 80,
              height: 80,
              backgroundColor: "#ffb74d",
              zIndex: 100,
            }}
          />
        </View>
        <View
          style={{
            position: "absolute",
            left: 30,
            top: 30,
            width: 80,
            height: 80,
            backgroundColor: "#16a085",
            zIndex: 1,
          }}
        />
      </View>
    </View>
  );
}
