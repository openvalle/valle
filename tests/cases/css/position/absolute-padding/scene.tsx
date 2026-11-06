export const composition = { width: 300, height: 220, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 300, height: 220, backgroundColor: "white" }}>
      <View
        key="outer"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 240,
          height: 160,
          backgroundColor: "#eef2f7",
          padding: 20,
          border: "4px solid #17324d",
        }}
      >
        <View
          key="static"
          style={{ width: 100, height: 80, padding: 10, backgroundColor: "#176bba" }}
        >
          <View
            key="placed"
            style={{
              position: "absolute",
              right: 10,
              bottom: 10,
              width: "50%",
              height: "50%",
              backgroundColor: "#ffb74d",
            }}
          />
        </View>
      </View>
    </View>
  );
}
