export const composition = { width: 204, height: 132, fps: 4, duration: 2 };

const CASES = [
  { root: {}, inner: "none", x: 8, y: 8, key: "c0" },
  { root: {}, inner: "invert(1)", x: 196, y: 8, key: "c1" },
  { root: { opacity: 0.65 }, inner: "none", x: 384, y: 8, key: "c2" },
  { root: { opacity: 0.65 }, inner: "invert(1)", x: 572, y: 8, key: "c3" },
  { root: { filter: "invert(1)" }, inner: "none", x: 8, y: 124, key: "c4" },
  { root: { filter: "invert(1)" }, inner: "invert(1)", x: 196, y: 124, key: "c5" },
  { root: { clipPath: "inset(6px)" }, inner: "none", x: 384, y: 124, key: "c6" },
  { root: { clipPath: "inset(6px)" }, inner: "invert(1)", x: 572, y: 124, key: "c7" },
  {
    root: { maskImage: "linear-gradient(black,transparent)" },
    inner: "none",
    x: 8,
    y: 240,
    key: "c8",
  },
  {
    root: { maskImage: "linear-gradient(black,transparent)" },
    inner: "invert(1)",
    x: 196,
    y: 240,
    key: "c9",
  },
  { root: { isolation: "isolate" }, inner: "none", x: 384, y: 240, key: "c10" },
  { root: { isolation: "isolate" }, inner: "invert(1)", x: 572, y: 240, key: "c11" },
];
const STATES = [0, 3, 5, 7, 9, 11, 0, 7];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const frame = Math.floor(seconds * 4);
  const active = { ...CASES[STATES[frame]], x: 8, y: 8, key: "sample" };
  return (
    <View style={{ width: 204, height: 132, background: "white" }}>
      {[active].map((c) => (
        <View
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 92,
            background: "linear-gradient(90deg,#2463a0 50%,#e0b060 50%)",
          }}
        >
          <View
            style={{ position: "absolute", left: 8, top: 8, width: 140, height: 76, ...c.root }}
          >
            <View
              style={{
                position: "absolute",
                left: 8,
                top: 8,
                width: 120,
                height: 56,
                background: "#80403080",
                backdropFilter: "invert(1)",
              }}
            >
              <View
                style={{
                  position: "absolute",
                  left: 16,
                  top: 8,
                  width: 80,
                  height: 40,
                  background: "#40a08080",
                  backdropFilter: c.inner,
                }}
              />
            </View>
          </View>
        </View>
      ))}
    </View>
  );
}
