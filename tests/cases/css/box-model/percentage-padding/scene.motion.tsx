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
          width: 260,
          height: 180,
          backgroundColor: "#eef2f7",
          padding: 10,
          border: "10px solid #17324d",
        }}
      >
        <View
          key="child"
          style={{
            width: "50%",
            height: "50%",
            padding: "10%",
            border: "2px solid #176bba",
            backgroundColor: "#16a085",
          }}
        />
      </View>
    </View>
  );
}
