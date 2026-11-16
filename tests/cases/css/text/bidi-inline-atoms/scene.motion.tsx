export const composition = { width: 768, height: 636, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Test: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    Han: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }),
    Hebrew: resource({ kind: "font", default: "/tests/resources/NotoSansHebrew.ttf" }),
  },
};
const CASES = [
  { writing: "horizontal-tb", direction: "ltr", align: "baseline", x: 8, y: 8, key: "c0" },
  { writing: "horizontal-tb", direction: "ltr", align: "middle", x: 196, y: 8, key: "c1" },
  { writing: "horizontal-tb", direction: "ltr", align: "top", x: 384, y: 8, key: "c2" },
  { writing: "horizontal-tb", direction: "rtl", align: "baseline", x: 572, y: 8, key: "c3" },
  { writing: "horizontal-tb", direction: "rtl", align: "middle", x: 8, y: 132, key: "c4" },
  { writing: "horizontal-tb", direction: "rtl", align: "top", x: 196, y: 132, key: "c5" },
  { writing: "vertical-rl", direction: "ltr", align: "baseline", x: 384, y: 132, key: "c6" },
  { writing: "vertical-rl", direction: "ltr", align: "middle", x: 572, y: 132, key: "c7" },
  { writing: "vertical-rl", direction: "ltr", align: "top", x: 8, y: 256, key: "c8" },
  { writing: "vertical-rl", direction: "rtl", align: "baseline", x: 196, y: 256, key: "c9" },
  { writing: "vertical-rl", direction: "rtl", align: "middle", x: 384, y: 256, key: "c10" },
  { writing: "vertical-rl", direction: "rtl", align: "top", x: 572, y: 256, key: "c11" },
  { writing: "vertical-lr", direction: "ltr", align: "baseline", x: 8, y: 380, key: "c12" },
  { writing: "vertical-lr", direction: "ltr", align: "middle", x: 196, y: 380, key: "c13" },
  { writing: "vertical-lr", direction: "ltr", align: "top", x: 384, y: 380, key: "c14" },
  { writing: "vertical-lr", direction: "rtl", align: "baseline", x: 572, y: 380, key: "c15" },
  { writing: "vertical-lr", direction: "rtl", align: "middle", x: 8, y: 504, key: "c16" },
  { writing: "vertical-lr", direction: "rtl", align: "top", x: 196, y: 504, key: "c17" },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 636, background: "white" }}>
      {CASES.map((c) => (
        <Text
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 108,
            fontFamily: "Test, Hebrew, Han",
            fontSize: 20,
            lineHeight: "34px",
            writingMode: c.writing,
            direction: c.direction,
            color: "#17365d",
            background: "#eef3f8",
          }}
        >
          AB אב
          <View
            style={{
              display: "inline-block",
              width: 24,
              height: 20,
              verticalAlign: c.align,
              background: "#e0b060",
            }}
          />
          中文
          <Span
            style={{
              fontSize: 14,
              textDecorationLine: "underline",
              textDecorationColor: "#c05c30",
            }}
          >
            {" "}
            cd
          </Span>
        </Text>
      ))}
    </View>
  );
}
