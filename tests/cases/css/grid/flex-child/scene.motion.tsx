export const composition = { width: 460, height: 220, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 460, height: 220, backgroundColor: "white" }}>
      <View
        key="outer"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 420,
          height: 180,
          backgroundColor: "#eef2f7",
          display: "grid",
          gridTemplateColumns: "180px 200px",
          gap: 20,
          padding: 10,
        }}
      >
        <View
          key="cell0"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            padding: 10,
            backgroundColor: "#dce5ef",
          }}
        >
          <View
            key="fixed"
            style={{ width: 30, height: 40, flexShrink: 0, backgroundColor: "#176bba" }}
          />
          <View key="grow" style={{ flex: "1 1 0px", height: 40, backgroundColor: "#16a085" }} />
        </View>
        <View
          key="cell1"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            padding: 10,
            backgroundColor: "#dce5ef",
          }}
        >
          <View
            key="fixed"
            style={{ width: 30, height: 40, flexShrink: 0, backgroundColor: "#176bba" }}
          />
          <View key="grow" style={{ flex: "1 1 0px", height: 40, backgroundColor: "#16a085" }} />
        </View>
      </View>
    </View>
  );
}
