export const composition = { width: 400, height: 300, fps: 4, duration: 2 };
export const controls = {
  resources: { CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }) },
};
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 400, height: 300, background: "white" }}>
      <Text
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 70,
          height: 250,
          fontFamily: "CJK",
          fontSize: 26,
          lineHeight: "44px",
          writingMode: "vertical-rl",
          color: "#17365d",
        }}
      >
        公元<Span style={{ textCombineUpright: "all", color: "#b91c1c" }}>2026</Span>年
        <Span style={{ textCombineUpright: "all" }}>12</Span>月
        <Span style={{ textCombineUpright: "all" }}>3</Span>日
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 110,
          top: 20,
          width: 70,
          height: 250,
          fontFamily: "CJK",
          fontSize: 26,
          lineHeight: "44px",
          writingMode: "vertical-rl",
          color: "#17365d",
        }}
      >
        编号<Span style={{ textCombineUpright: "all" }}>2026</Span>组
        <Span style={{ textCombineUpright: "all" }}>123</Span>号
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 200,
          top: 20,
          width: 70,
          height: 250,
          fontFamily: "CJK",
          fontSize: 26,
          lineHeight: "44px",
          writingMode: "vertical-lr",
          color: "#17365d",
          textDecoration: "underline",
          textUnderlineOffset: "3px",
        }}
      >
        第<Span style={{ textCombineUpright: seconds < 1 ? "all" : "none" }}>１２</Span>期
        <Span style={{ textCombineUpright: "all" }}>ABC</Span>
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 290,
          top: 20,
          width: 70,
          height: 250,
          fontFamily: "CJK",
          fontSize: 26,
          lineHeight: "60px",
          writingMode: "vertical-rl",
          color: "#17365d",
          textEmphasis: "'\u2022' crimson",
        }}
      >
        第<Span style={{ textCombineUpright: "all" }}>12</Span>期
      </Text>
    </View>
  );
}
