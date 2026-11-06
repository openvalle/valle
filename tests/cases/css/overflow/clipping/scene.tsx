export const composition = { width: 340, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 160, backgroundColor: "white" }}>
      <View
        key="hidden"
        style={{
          position: "absolute",
          left: 30,
          top: 30,
          width: 120,
          height: 100,
          backgroundColor: "#eef2f7",
          overflow: "hidden",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: -20,
            top: 20,
            width: 160,
            height: 40,
            backgroundColor: "#176bba",
          }}
        />
      </View>
      <View
        key="visible"
        style={{
          position: "absolute",
          left: 190,
          top: 30,
          width: 120,
          height: 100,
          backgroundColor: "#eef2f7",
          overflow: "visible",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: -20,
            top: 20,
            width: 160,
            height: 40,
            backgroundColor: "#176bba",
          }}
        />
      </View>
    </View>
  );
}
