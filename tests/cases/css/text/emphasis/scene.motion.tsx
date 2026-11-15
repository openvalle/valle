export const composition = { width: 460, height: 360, fps: 4, duration: 2 };
export const controls = {
  resources: {
    Sans: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }),
  },
};
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 460, height: 360, background: "white" }}>
      <Text
        style={{
          position: "absolute",
          left: 24,
          top: 24,
          fontFamily: "Sans, CJK",
          fontSize: 32,
          lineHeight: "60px",
          color: "#152c44",
          textEmphasisStyle: seconds < 1 ? "filled dot" : "open triangle",
          textEmphasisColor: "#b91c1c",
          textEmphasisPosition: "over",
        }}
      >
        Abc 123, !?
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 24,
          top: 102,
          fontFamily: "Sans, CJK",
          fontSize: 32,
          lineHeight: "60px",
          color: "#152c44",
          textEmphasisStyle: "open circle",
          textEmphasisColor: "#2563eb",
          textEmphasisPosition: "under",
        }}
      >
        DEF abc.
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 24,
          top: 180,
          fontFamily: "Sans, CJK",
          fontSize: 32,
          lineHeight: "60px",
          color: "#152c44",
          textEmphasisStyle: "'\u2605'",
          textEmphasisColor: "currentColor",
          textEmphasisPosition: "over",
        }}
      >
        Go! #50%
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 24,
          top: 258,
          fontFamily: "Sans, CJK",
          fontSize: 32,
          lineHeight: "60px",
          color: "#152c44",
          textEmphasisStyle: "filled triangle",
          textEmphasisColor: "#15803d",
          textEmphasisPosition: "over",
        }}
      >
        office
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 300,
          top: 28,
          width: 60,
          height: 300,
          fontFamily: "CJK",
          fontSize: 30,
          lineHeight: "60px",
          writingMode: "vertical-rl",
          color: "#152c44",
          textEmphasis: "filled dot #b91c1c",
          textEmphasisPosition: "over right",
        }}
      >
        春风，山水
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 380,
          top: 28,
          width: 60,
          height: 300,
          fontFamily: "CJK",
          fontSize: 30,
          lineHeight: "60px",
          writingMode: "vertical-rl",
          color: "#152c44",
          textEmphasis: "filled dot #b91c1c",
          textEmphasisPosition: "over left",
        }}
      >
        春风，山水
      </Text>
    </View>
  );
}
