export const composition = { width: 768, height: 364, fps: 1, duration: 1 };

const CASES = [
  { display: "block", height: "auto", min: 24, x: 8, y: 8, key: "c0" },
  { display: "block", height: "auto", min: 48, x: 196, y: 8, key: "c1" },
  { display: "block", height: 100, min: 24, x: 384, y: 8, key: "c2" },
  { display: "block", height: 100, min: 48, x: 572, y: 8, key: "c3" },
  { display: "flex", height: "auto", min: 24, x: 8, y: 124, key: "c4" },
  { display: "flex", height: "auto", min: 48, x: 196, y: 124, key: "c5" },
  { display: "flex", height: 100, min: 24, x: 384, y: 124, key: "c6" },
  { display: "flex", height: 100, min: 48, x: 572, y: 124, key: "c7" },
  { display: "grid", height: "auto", min: 24, x: 8, y: 240, key: "c8" },
  { display: "grid", height: "auto", min: 48, x: 196, y: 240, key: "c9" },
  { display: "grid", height: 100, min: 24, x: 384, y: 240, key: "c10" },
  { display: "grid", height: 100, min: 48, x: 572, y: 240, key: "c11" },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 364, background: "white" }}>
      {CASES.map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 160,
            height: c.height,
            padding: 4,
            border: "2px solid #17365d",
            display: c.display,
            alignItems: "start",
            background: "#eef3f8",
          }}
        >
          <View
            key="item"
            style={{
              width: "50%",
              height: "50%",
              minWidth: 90,
              maxWidth: 100,
              minHeight: c.min,
              maxHeight: 64,
              background: "#2463a0",
            }}
          >
            <View style={{ width: 24, height: 40, background: "#e0b060" }} />
          </View>
        </View>
      ))}
    </View>
  );
}
