export const composition = { width: 768, height: 312, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Test: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    Han: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }),
    Hebrew: resource({ kind: "font", default: "/tests/resources/NotoSansHebrew.ttf" }),
  },
};
const CASES = [
  { axis: "normal", synthesis: "none", adjust: "none", x: 8, y: 8, key: "c0" },
  { axis: "normal", synthesis: "none", adjust: 0.7, x: 196, y: 8, key: "c1" },
  { axis: "normal", synthesis: "weight style", adjust: "none", x: 384, y: 8, key: "c2" },
  { axis: "normal", synthesis: "weight style", adjust: 0.7, x: 572, y: 8, key: "c3" },
  { axis: '"wght" 750, "wdth" 75', synthesis: "none", adjust: "none", x: 8, y: 156, key: "c4" },
  { axis: '"wght" 750, "wdth" 75', synthesis: "none", adjust: 0.7, x: 196, y: 156, key: "c5" },
  {
    axis: '"wght" 750, "wdth" 75',
    synthesis: "weight style",
    adjust: "none",
    x: 384,
    y: 156,
    key: "c6",
  },
  {
    axis: '"wght" 750, "wdth" 75',
    synthesis: "weight style",
    adjust: 0.7,
    x: 572,
    y: 156,
    key: "c7",
  },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 312, background: "white" }}>
      {CASES.map((c) => (
        <Text
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 176,
            height: 90,
            fontFamily: "Test",
            fontSize: 24,
            lineHeight: "42px",
            fontStyle: "oblique 12deg",
            fontWeight: 500,
            fontVariationSettings: c.axis,
            fontSynthesis: c.synthesis,
            fontSizeAdjust: c.adjust,
            color: "#17365d",
          }}
        >
          Axis 0123<Span style={{ fontWeight: "bolder", fontSize: "smaller" }}>Axis 0123</Span>
        </Text>
      ))}
    </View>
  );
}
