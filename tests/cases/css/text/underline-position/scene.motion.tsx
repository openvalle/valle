export const composition = { width: 500, height: 240, fps: 4, duration: 2 };
export const controls = {
  resources: { CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }) },
};
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 500, height: 240, background: "white" }}>
      <Text
        style={{
          position: "absolute",
          left: 20,
          top: 10,
          fontFamily: "CJK",
          fontSize: 28,
          lineHeight: "46px",
          color: "#18385d",
          textDecoration: "underline 2px",
          textDecorationSkipInk: "none",
          textUnderlinePosition: "auto",
          writingMode: "horizontal-tb",
        }}
      >
        gyp Auto
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 20,
          top: 65,
          fontFamily: "CJK",
          fontSize: 28,
          lineHeight: "46px",
          color: "#18385d",
          textDecoration: "underline 2px",
          textDecorationSkipInk: "none",
          textUnderlinePosition: "under",
          writingMode: "horizontal-tb",
        }}
      >
        gyp Under
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 20,
          top: 125,
          fontFamily: "CJK",
          fontSize: 28,
          lineHeight: "46px",
          color: "#18385d",
          textDecoration: "underline 2px",
          textDecorationSkipInk: "none",
          textUnderlinePosition: "under",
          writingMode: "horizontal-tb",
          textUnderlineOffset: "5px",
        }}
      >
        gyp Offset
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 300,
          top: 15,
          fontFamily: "CJK",
          fontSize: 28,
          lineHeight: "46px",
          color: "#18385d",
          textDecoration: "underline 2px",
          textDecorationSkipInk: "none",
          textUnderlinePosition: "left",
          writingMode: "vertical-rl",
          width: 50,
          height: 200,
        }}
      >
        春山水
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 360,
          top: 15,
          fontFamily: "CJK",
          fontSize: 28,
          lineHeight: "46px",
          color: "#18385d",
          textDecoration: "underline 2px",
          textDecorationSkipInk: "none",
          textUnderlinePosition: "right",
          writingMode: "vertical-rl",
          width: 50,
          height: 200,
        }}
      >
        春山水
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 420,
          top: 15,
          fontFamily: "CJK",
          fontSize: 28,
          lineHeight: "46px",
          color: "#18385d",
          textDecoration: "underline 2px",
          textDecorationSkipInk: "none",
          textUnderlinePosition: "under right",
          writingMode: "vertical-lr",
          width: 50,
          height: 200,
        }}
      >
        春山水
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 20,
          top: 185,
          fontSize: 24,
          fontFamily: "CJK",
          textDecoration: "underline 2px",
          textDecorationSkipInk: "none",
          textUnderlinePosition: seconds < 1 ? "under" : "auto",
        }}
      >
        small <Span style={{ fontSize: 36 }}>gyp</Span>
      </Text>
    </View>
  );
}
