export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 320, height: 200, backgroundColor: "white" }}>
      <View
        key="wrap"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 220,
          height: 160,
          backgroundColor: "#eef2f7",
          display: "flex",
          flexWrap: "wrap",
          alignContent: "flex-start",
          gap: "12px 20px",
        }}
      >
        <View
          key="0"
          style={{ width: 100, height: 40, flexShrink: 0, backgroundColor: "#176bba" }}
        />
        <View
          key="1"
          style={{ width: 100, height: 40, flexShrink: 0, backgroundColor: "#16a085" }}
        />
        <View
          key="2"
          style={{ width: 100, height: 40, flexShrink: 0, backgroundColor: "#ffb74d" }}
        />
        <View
          key="3"
          style={{ width: 100, height: 40, flexShrink: 0, backgroundColor: "#ad1457" }}
        />
      </View>
    </View>
  );
}
