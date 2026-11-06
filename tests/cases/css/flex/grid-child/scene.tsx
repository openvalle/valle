export const composition = { width: 464, height: 244, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 464, height: 244, backgroundColor: "white" }}>
      <View
        key="outer"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 424,
          height: 204,
          backgroundColor: "#eef2f7",
          display: "flex",
          gap: 20,
          padding: 10,
          border: "2px solid #17324d",
        }}
      >
        <View key="side" style={{ width: 100, flexShrink: 0, backgroundColor: "#176bba" }} />
        <View
          key="grid"
          style={{
            display: "grid",
            flex: "1 1 0px",
            minWidth: 0,
            gridTemplateColumns: "1fr 1fr",
            gridTemplateRows: "1fr 1fr",
            columnGap: 12,
            rowGap: 10,
            padding: 8,
            border: "2px solid #17324d",
          }}
        >
          <View key="cell0" style={{ backgroundColor: "#16a085" }} />
          <View key="cell1" style={{ backgroundColor: "#ffb74d" }} />
          <View key="cell2" style={{ backgroundColor: "#176bba" }} />
          <View key="cell3" style={{ backgroundColor: "#a8135d" }} />
        </View>
      </View>
    </View>
  );
}
