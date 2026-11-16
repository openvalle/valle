export const composition = { width: 768, height: 248, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Test: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    Han: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }),
    Hebrew: resource({ kind: "font", default: "/tests/resources/NotoSansHebrew.ttf" }),
  },
};
const CASES = [
  { weight: 100, size: 16, x: 8, y: 8, key: "c0" },
  { weight: 100, size: 24, x: 196, y: 8, key: "c1" },
  { weight: 400, size: 16, x: 384, y: 8, key: "c2" },
  { weight: 400, size: 24, x: 572, y: 8, key: "c3" },
  { weight: 600, size: 16, x: 8, y: 124, key: "c4" },
  { weight: 600, size: 24, x: 196, y: 124, key: "c5" },
  { weight: 900, size: 16, x: 384, y: 124, key: "c6" },
  { weight: 900, size: 24, x: 572, y: 124, key: "c7" },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 248, background: "white" }}>
      {CASES.map((c) => (
        <Text
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 176,
            height: 108,
            fontFamily: "Test",
            fontSize: c.size,
            lineHeight: "34px",
            fontWeight: c.weight,
            color: "#17365d",
          }}
        >
          Base<Span style={{ fontSize: "larger", fontWeight: "bolder" }}> Large</Span>
          <Span style={{ fontSize: "smaller", fontWeight: "lighter" }}> small</Span>
        </Text>
      ))}
    </View>
  );
}
