export const composition = { width: 340, height: 160, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 340, height: 160, backgroundColor: "white" }}>
      <View
        key="row"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 300,
          height: 120,
          backgroundColor: "#eef2f7",
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <View key="last" style={{ width: 40, height: 40, order: 2, backgroundColor: "#176bba" }} />
        <View key="first" style={{ width: 60, height: 60, order: 0, backgroundColor: "#16a085" }} />
        <View
          key="middle"
          style={{
            width: 80,
            height: 80,
            order: 1,
            alignSelf: "flex-end",
            backgroundColor: "#ffb74d",
          }}
        />
      </View>
    </View>
  );
}
