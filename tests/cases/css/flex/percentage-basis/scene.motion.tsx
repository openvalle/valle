export const composition = { width: 380, height: 240, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 380, height: 240, backgroundColor: "white" }}>
      <View
        key="row0"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 320,
          height: 80,
          backgroundColor: "#eef2f7",
          display: "flex",
          padding: 10,
          gap: 20,
        }}
      >
        <View
          key="item0"
          style={{ flexBasis: "50%", flexShrink: 1, height: 60, backgroundColor: "#176bba" }}
        />
        <View
          key="item1"
          style={{ flexBasis: "50%", flexShrink: 1, height: 60, backgroundColor: "#16a085" }}
        />
      </View>
      <View
        key="row1"
        style={{
          position: "absolute",
          left: 20,
          top: 130,
          width: 320,
          height: 80,
          backgroundColor: "#eef2f7",
          display: "flex",
          padding: 10,
          gap: 20,
        }}
      >
        <View
          key="item0"
          style={{ flexBasis: "50%", flexShrink: 0, height: 60, backgroundColor: "#176bba" }}
        />
        <View
          key="item1"
          style={{ flexBasis: "50%", flexShrink: 0, height: 60, backgroundColor: "#16a085" }}
        />
      </View>
    </View>
  );
}
