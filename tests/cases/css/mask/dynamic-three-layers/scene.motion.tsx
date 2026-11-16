export const composition = { width: 204, height: 132, fps: 4, duration: 2 };

const CASES = [
  { composite: "add", mode: "alpha", x: 8, y: 8, key: "c0" },
  { composite: "add", mode: "luminance", x: 196, y: 8, key: "c1" },
  { composite: "subtract", mode: "alpha", x: 384, y: 8, key: "c2" },
  { composite: "subtract", mode: "luminance", x: 572, y: 8, key: "c3" },
  { composite: "intersect", mode: "alpha", x: 8, y: 124, key: "c4" },
  { composite: "intersect", mode: "luminance", x: 196, y: 124, key: "c5" },
  { composite: "exclude", mode: "alpha", x: 384, y: 124, key: "c6" },
  { composite: "exclude", mode: "luminance", x: 572, y: 124, key: "c7" },
];
const STATES = [0, 1, 2, 3, 5, 7, 0, 3];
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
            background: "#c05080",
            maskImage:
              "linear-gradient(90deg,black 60%,transparent 60%),linear-gradient(white 65%,transparent 65%),linear-gradient(90deg,transparent 20%,#808080 20%)",
            maskComposite: c.composite,
            maskMode: c.mode,
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 36,
              top: 16,
              width: 100,
              height: 60,
              background: "#40a080",
            }}
          />
        </View>
      ))}
    </View>
  );
}
