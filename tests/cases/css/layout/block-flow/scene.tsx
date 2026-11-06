export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 320, height: 200, backgroundColor: "white" }}>
      <View
        key="flow"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 280,
          height: 160,
          backgroundColor: "#eef2f7",
          padding: 10,
          border: "2px solid #102a43",
        }}
      >
        <View key="full" style={{ height: 30, backgroundColor: "#176bba" }} />
        <View
          key="center"
          style={{
            width: 100,
            height: 40,
            marginLeft: "auto",
            marginRight: "auto",
            backgroundColor: "#16a085",
          }}
        />
        <View key="last" style={{ height: 30, backgroundColor: "#ffb74d" }} />
      </View>
    </View>
  );
}
