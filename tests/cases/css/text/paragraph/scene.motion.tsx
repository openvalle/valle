export const composition = { width: 490, height: 205, fps: 4, duration: 2 };
export const controls = {
  resources: { CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }) },
};
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 490, height: 205, background: "white" }}>
      <Text
        key="last"
        style={{
          position: "absolute",
          fontFamily: "sans-serif",
          fontSize: 20,
          lineHeight: "28px",
          color: "#17324d",
          backgroundColor: "#eff6ff",
          left: 10,
          top: 10,
          width: 260,
          height: 100,
          whiteSpace: "pre-wrap",
          textAlign: "justify",
          textAlignLast: seconds < 1 ? "right" : "center",
        }}
      >
        {"One two three four five six seven\nEnd"}
      </Text>
      <Text
        key="pretty"
        style={{
          position: "absolute",
          fontFamily: "sans-serif",
          fontSize: 20,
          lineHeight: "28px",
          color: "#17324d",
          backgroundColor: "#eff6ff",
          left: 10,
          top: 120,
          width: 260,
          height: 70,
          textWrap: "pretty",
        }}
      >
        {"One two three four five six seven"}
      </Text>
      <Text
        key="anywhere"
        style={{
          position: "absolute",
          fontFamily: "sans-serif",
          fontSize: 20,
          lineHeight: "28px",
          color: "#17324d",
          backgroundColor: "#eff6ff",
          left: 280,
          top: 10,
          width: 70,
          height: 180,
          lineBreak: "anywhere",
          wordBreak: "keep-all",
        }}
      >
        {"ABC DEFGH IJKLMNO"}
      </Text>
      <Text
        key="strict"
        style={{
          position: "absolute",
          fontFamily: "CJK",
          fontSize: 20,
          lineHeight: "28px",
          color: "#17324d",
          backgroundColor: "#eff6ff",
          left: 360,
          top: 10,
          width: 52,
          height: 180,
          lineBreak: "strict",
        }}
      >
        {"春あぁ天あぁ山"}
      </Text>
      <Text
        key="loose"
        style={{
          position: "absolute",
          fontFamily: "CJK",
          fontSize: 20,
          lineHeight: "28px",
          color: "#17324d",
          backgroundColor: "#eff6ff",
          left: 420,
          top: 10,
          width: 52,
          height: 180,
          lineBreak: "loose",
        }}
      >
        {"春あぁ天あぁ山"}
      </Text>
    </View>
  );
}
