export const composition = { width: 768, height: 248, fps: 1, duration: 1 };

const CASES = [
  { writing: "horizontal-tb", direction: "ltr", x: 8, y: 8, key: "c0" },
  { writing: "horizontal-tb", direction: "rtl", x: 196, y: 8, key: "c1" },
  { writing: "vertical-rl", direction: "ltr", x: 384, y: 8, key: "c2" },
  { writing: "vertical-rl", direction: "rtl", x: 572, y: 8, key: "c3" },
  { writing: "vertical-lr", direction: "ltr", x: 8, y: 124, key: "c4" },
  { writing: "vertical-lr", direction: "rtl", x: 196, y: 124, key: "c5" },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 248, background: "white" }}>
      {CASES.map((c) => (
        <View
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 92,
            writingMode: c.writing,
            direction: c.direction,
            border: "5px solid #2463a0",
            borderStartStartRadius: 28,
            borderStartEndRadius: 4,
            borderEndStartRadius: 12,
            borderEndEndRadius: 20,
            background: "#e0b060",
          }}
        />
      ))}
    </View>
  );
}
