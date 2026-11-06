export const composition = { width: 260, height: 210, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 260, height: 210, backgroundColor: "white" }}>
      <View
        key="grid"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 190,
          height: 160,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "[start] 80px [mid] 100px [end]",
          gridTemplateRows: "40px",
          gridAutoRows: "30px 50px",
          gap: 10,
        }}
      >
        <View
          key="span"
          style={{ gridColumn: "start / end", gridRow: 1, backgroundColor: "#176bba" }}
        />
        <View
          key="implicit"
          style={{ gridColumn: "mid", gridRow: 3, backgroundColor: "#16a085" }}
        />
      </View>
    </View>
  );
}
