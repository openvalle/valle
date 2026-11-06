export const composition = { width: 320, height: 210, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 320, height: 210, backgroundColor: "white" }}>
      <View
        key="grid"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 280,
          height: 170,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "repeat(3, 80px)",
          gridTemplateRows: "repeat(3, 50px)",
          gap: 10,
        }}
      >
        <View
          key="span"
          style={{ gridColumn: "2 / span 2", gridRow: 1, backgroundColor: "#176bba" }}
        />
        <View
          key="tall"
          style={{ gridColumn: 1, gridRow: "1 / span 2", backgroundColor: "#16a085" }}
        />
        <View
          key="aligned"
          style={{
            gridColumn: 3,
            gridRow: 3,
            justifySelf: "center",
            alignSelf: "center",
            width: 40,
            height: 20,
            backgroundColor: "#ffb74d",
          }}
        />
      </View>
    </View>
  );
}
