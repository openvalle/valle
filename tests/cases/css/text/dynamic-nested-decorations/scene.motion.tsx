export const composition = { width: 204, height: 148, fps: 4, duration: 2 };
export const controls = {
  resources: {
    Test: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    Han: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }),
    Hebrew: resource({ kind: "font", default: "/tests/resources/NotoSansHebrew.ttf" }),
  },
};
const CASES = [
  { dec: "underline", overflow: "visible", x: 8, y: 8, key: "c0" },
  { dec: "underline", overflow: "hidden", x: 196, y: 8, key: "c1" },
  { dec: "overline line-through", overflow: "visible", x: 384, y: 8, key: "c2" },
  { dec: "overline line-through", overflow: "hidden", x: 572, y: 8, key: "c3" },
];
const STATES = [0, 1, 2, 3, 1, 0, 3, 0];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const frame = Math.floor(seconds * 4);
  const active = { ...CASES[STATES[frame]], x: 8, y: 8, key: "sample" };
  return (
    <View style={{ width: 204, height: 148, background: "white" }}>
      {[active].map((c) => (
        <Text
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 108,
            fontFamily: "Test",
            fontSize: 24,
            lineHeight: "36px",
            color: "#17365d",
            textDecorationLine: c.dec,
            textDecorationColor: "#2463a0",
            overflow: c.overflow,
          }}
        >
          outer{" "}
          <Span
            style={{
              fontSize: 16,
              color: "#3b856c",
              textDecorationLine: "underline",
              textDecorationStyle: "double",
              textDecorationColor: "#c05c30",
            }}
          >
            inner <Span style={{ fontSize: 28, color: "#8c407c" }}>nested</Span>
          </Span>{" "}
          end
        </Text>
      ))}
    </View>
  );
}
