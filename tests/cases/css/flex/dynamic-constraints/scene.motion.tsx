export const composition = { width: 300, height: 210, fps: 4, duration: 2 };
const WIDTHS = [118, 120, 122, 118, 132, 120, 118, 122];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const i = Math.floor(seconds * 4);
  const width = WIDTHS[i];
  return (
    <View key="canvas" style={{ width: 300, height: 210, background: "white" }}>
      <View
        key="wrap"
        style={{
          position: "absolute",
          left: 12,
          top: 12,
          width,
          height: 76,
          display: "flex",
          flexWrap: "wrap",
          alignContent: "start",
          gap: "4px 0px",
          background: "#eef3f8",
        }}
      >
        <View key="a" style={{ width: 60, height: 24, flexShrink: 0, background: "#d040a0" }} />
        <View key="b" style={{ width: 60, height: 24, flexShrink: 0, background: "#40a0c0" }} />
      </View>
      <View
        key="nested"
        style={{
          position: "absolute",
          left: 12,
          top: 108,
          width: width + 40,
          height: 76,
          padding: 6,
          display: i % 2 ? "grid" : "flex",
          gridTemplateColumns: "1fr",
          background: "#eef3f8",
        }}
      >
        <View
          key="inner"
          style={{
            width: "50%",
            minWidth: 90,
            maxWidth: 110,
            height: 48,
            display: "flex",
            background: "#ffd080",
          }}
        >
          <View key="fill" style={{ width: "50%", height: 24, background: "#4070c0" }} />
        </View>
      </View>
    </View>
  );
}
