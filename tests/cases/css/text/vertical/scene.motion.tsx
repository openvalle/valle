export const composition = { width: 420, height: 260, fps: 4, duration: 2 };
export const controls = {
  resources: { CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }) },
};
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 420, height: 260, background: "white" }}>
      <Text
        key="mixed"
        style={{
          position: "absolute",
          left: 10,
          top: 10,
          width: 90,
          height: 230,
          fontFamily: "CJK",
          fontSize: 24,
          lineHeight: "32px",
          writingMode: seconds < 1 ? "vertical-rl" : "vertical-lr",
          color: "#17365d",
          background: "#eff6ff",
        }}
      >
        春风<Span style={{ color: "#b91c1c" }}>ABC</Span>，（山水）。天地人间
      </Text>
      <Text
        key="lr"
        style={{
          position: "absolute",
          left: 110,
          top: 10,
          width: 90,
          height: 230,
          fontFamily: "CJK",
          fontSize: 24,
          lineHeight: "32px",
          writingMode: "vertical-lr",
          color: "#14532d",
          background: "#f0fdf4",
        }}
      >
        春风<Span style={{ color: "#b91c1c" }}>ABC</Span>，（山水）。天地人间
      </Text>
      <Text
        key="upright"
        style={{
          position: "absolute",
          left: 210,
          top: 10,
          width: 90,
          height: 230,
          fontFamily: "CJK",
          fontSize: 24,
          lineHeight: "32px",
          writingMode: "vertical-rl",
          textOrientation: "upright",
          color: "#7c2d12",
          background: "#fff7ed",
        }}
      >
        Valle 2026
      </Text>
      <Text
        key="sideways"
        style={{
          position: "absolute",
          left: 310,
          top: 10,
          width: 90,
          height: 230,
          fontFamily: "CJK",
          fontSize: 24,
          lineHeight: "32px",
          writingMode: "vertical-rl",
          textOrientation: "sideways",
          color: "#581c87",
          background: "#faf5ff",
          textDecoration: "underline 1px",
          whiteSpace: "pre",
        }}
      >
        {"Valle\nMotion"}
      </Text>
    </View>
  );
}
