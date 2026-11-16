export const composition = { width: 204, height: 132, fps: 4, duration: 2 };

const CASES = [
  { size: 16, unit: "em", writing: "horizontal-tb", width: "2em", x: 8, y: 8, key: "c0" },
  { size: 16, unit: "em", writing: "vertical-rl", width: "2em", x: 196, y: 8, key: "c1" },
  { size: 16, unit: "em", writing: "vertical-lr", width: "2em", x: 384, y: 8, key: "c2" },
  { size: 16, unit: "rem", writing: "horizontal-tb", width: "2rem", x: 572, y: 8, key: "c3" },
  { size: 16, unit: "rem", writing: "vertical-rl", width: "2rem", x: 8, y: 124, key: "c4" },
  { size: 16, unit: "rem", writing: "vertical-lr", width: "2rem", x: 196, y: 124, key: "c5" },
  { size: 16, unit: "px", writing: "horizontal-tb", width: "32px", x: 384, y: 124, key: "c6" },
  { size: 16, unit: "px", writing: "vertical-rl", width: "32px", x: 572, y: 124, key: "c7" },
  { size: 16, unit: "px", writing: "vertical-lr", width: "32px", x: 8, y: 240, key: "c8" },
  { size: 16, unit: "percent", writing: "horizontal-tb", width: "25%", x: 196, y: 240, key: "c9" },
  { size: 16, unit: "percent", writing: "vertical-rl", width: "25%", x: 384, y: 240, key: "c10" },
  { size: 16, unit: "percent", writing: "vertical-lr", width: "25%", x: 572, y: 240, key: "c11" },
  { size: 24, unit: "em", writing: "horizontal-tb", width: "2em", x: 8, y: 356, key: "c12" },
  { size: 24, unit: "em", writing: "vertical-rl", width: "2em", x: 196, y: 356, key: "c13" },
  { size: 24, unit: "em", writing: "vertical-lr", width: "2em", x: 384, y: 356, key: "c14" },
  { size: 24, unit: "rem", writing: "horizontal-tb", width: "2rem", x: 572, y: 356, key: "c15" },
  { size: 24, unit: "rem", writing: "vertical-rl", width: "2rem", x: 8, y: 472, key: "c16" },
  { size: 24, unit: "rem", writing: "vertical-lr", width: "2rem", x: 196, y: 472, key: "c17" },
  { size: 24, unit: "px", writing: "horizontal-tb", width: "32px", x: 384, y: 472, key: "c18" },
  { size: 24, unit: "px", writing: "vertical-rl", width: "32px", x: 572, y: 472, key: "c19" },
  { size: 24, unit: "px", writing: "vertical-lr", width: "32px", x: 8, y: 588, key: "c20" },
  { size: 24, unit: "percent", writing: "horizontal-tb", width: "25%", x: 196, y: 588, key: "c21" },
  { size: 24, unit: "percent", writing: "vertical-rl", width: "25%", x: 384, y: 588, key: "c22" },
  { size: 24, unit: "percent", writing: "vertical-lr", width: "25%", x: 572, y: 588, key: "c23" },
];
const STATES = [0, 12, 15, 23, 9, 0, 23, 12];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const frame = Math.floor(seconds * 4);
  const active = { ...CASES[STATES[frame]], x: 8, y: 8, key: "sample" };
  return (
    <View style={{ width: 204, height: 132, background: "white" }}>
      {[active].map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 160,
            height: 88,
            fontSize: c.size,
            writingMode: c.writing,
            background: "#eef3f8",
          }}
        >
          <View
            key="item"
            style={{
              position: "absolute",
              left: 8,
              top: 8,
              width: c.width,
              height: "2em",
              padding: "0.25em",
              background: "#2463a0",
            }}
          >
            <View style={{ width: "100%", height: "100%", background: "#e0b060" }} />
          </View>
        </View>
      ))}
    </View>
  );
}
