export const composition = { width: 400, height: 240, fps: 4, duration: 2 };
const WIDTHS = [86, 88, 86, 88, 86, 88, 86, 88];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const i = Math.floor(seconds * 4);
  const small = i >= 4;
  return (
    <View
      key="canvas"
      style={{
        width: 400,
        height: 240,
        background: "white",
        fontSize: small ? 16 : 24,
        lineHeight: small ? "24px" : "32px",
        color: i % 2 ? "#9d3c16" : "#17365d",
      }}
    >
      <Text
        key="wrap"
        style={{
          position: "absolute",
          left: 12,
          top: 12,
          width: WIDTHS[i],
          height: 112,
          overflowWrap: "anywhere",
          background: "#eef3f8",
        }}
      >
        MMMMMMMM
      </Text>
      <Text
        key="ellipsis"
        style={{
          position: "absolute",
          left: 12,
          top: 144,
          width: 120,
          height: 64,
          overflow: "hidden",
          whiteSpace: "nowrap",
          textOverflow: "ellipsis",
          direction: i % 2 ? "rtl" : "ltr",
          background: "#eef3f8",
        }}
      >
        ALPHA BETA GAMMA
      </Text>
      <Text
        key="vertical"
        style={{
          position: "absolute",
          left: 180,
          top: 12,
          width: 160,
          height: 200,
          writingMode: i % 2 ? "vertical-lr" : "vertical-rl",
          fontSize: 24,
          lineHeight: "36px",
          background: "#eef3f8",
        }}
      >
        Ag<Span style={{ fontSize: small ? 16 : 24, color: "#9d3c16" }}>pq</Span>
      </Text>
    </View>
  );
}
