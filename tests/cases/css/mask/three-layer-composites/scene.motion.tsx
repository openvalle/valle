export const composition = { width: 768, height: 248, fps: 1, duration: 1 };

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
