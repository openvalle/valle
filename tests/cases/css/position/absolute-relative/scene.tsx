export const composition = { width: 340, height: 220, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 220, backgroundColor: "white" }}>
      <View
        key="parent"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 300,
          height: 180,
          backgroundColor: "#eef2f7",
          border: "4px solid #102a43",
          padding: 10,
        }}
      >
        <View
          key="stretch"
          style={{
            position: "absolute",
            left: 10,
            right: 20,
            top: 12,
            height: 30,
            backgroundColor: "#176bba",
          }}
        />
        <View
          key="relative"
          style={{
            position: "relative",
            left: 16,
            top: 70,
            width: 100,
            height: 40,
            backgroundColor: "#16a085",
          }}
        />
        <View key="flow" style={{ width: 80, height: 30, backgroundColor: "#ffb74d" }} />
      </View>
    </View>
  );
}
