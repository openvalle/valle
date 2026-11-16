export const composition = { width: 768, height: 248, fps: 1, duration: 1 };
export const controls = {
  resources: { Test: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }) },
};
const CASES = [
  { filter: "none", transform: "none", position: "left top", x: 8, y: 8, key: "c0" },
  { filter: "none", transform: "none", position: "right bottom", x: 196, y: 8, key: "c1" },
  { filter: "none", transform: "rotate(6deg)", position: "left top", x: 384, y: 8, key: "c2" },
  { filter: "none", transform: "rotate(6deg)", position: "right bottom", x: 572, y: 8, key: "c3" },
  { filter: "blur(1px)", transform: "none", position: "left top", x: 8, y: 124, key: "c4" },
  { filter: "blur(1px)", transform: "none", position: "right bottom", x: 196, y: 124, key: "c5" },
  {
    filter: "blur(1px)",
    transform: "rotate(6deg)",
    position: "left top",
    x: 384,
    y: 124,
    key: "c6",
  },
  {
    filter: "blur(1px)",
    transform: "rotate(6deg)",
    position: "right bottom",
    x: 572,
    y: 124,
    key: "c7",
  },
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
            width: 160,
            height: 90,
            fontFamily: "Test",
            fontSize: 40,
            lineHeight: "64px",
            fontWeight: 700,
            color: "transparent",
            backgroundImage: "linear-gradient(90deg,#2463a0,#e0b060)",
            backgroundClip: "text",
            backgroundSize: "90px 70px",
            backgroundRepeat: "repeat-x",
            backgroundPosition: c.position,
            filter: c.filter,
            transform: c.transform,
          }}
        >
          MASK
        </Text>
      ))}
    </View>
  );
}
