export const composition = { width: 300, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 300, height: 160, backgroundColor: "white" }}>
      <View
        key="border"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 120,
          height: 100,
          backgroundColor: "#176bba",
          boxSizing: "border-box",
          border: "4px solid #102a43",
          padding: 10,
        }}
      />
      <View
        key="content"
        style={{
          position: "absolute",
          left: 170,
          top: 20,
          width: 80,
          height: 60,
          backgroundColor: "#ffb74d",
          boxSizing: "content-box",
          border: "4px solid #102a43",
          padding: 10,
        }}
      />
    </View>
  );
}
