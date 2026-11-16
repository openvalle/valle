export const composition = { width: 768, height: 436, fps: 1, duration: 1 };

const CASES = [
  { origin: "left top", flat: "flat", overflow: "visible", x: 8, y: 8, key: "c0" },
  { origin: "left top", flat: "flat", overflow: "hidden", x: 196, y: 8, key: "c1" },
  { origin: "left top", flat: "preserve-3d", overflow: "visible", x: 384, y: 8, key: "c2" },
  { origin: "left top", flat: "preserve-3d", overflow: "hidden", x: 572, y: 8, key: "c3" },
  { origin: "center", flat: "flat", overflow: "visible", x: 8, y: 148, key: "c4" },
  { origin: "center", flat: "flat", overflow: "hidden", x: 196, y: 148, key: "c5" },
  { origin: "center", flat: "preserve-3d", overflow: "visible", x: 384, y: 148, key: "c6" },
  { origin: "center", flat: "preserve-3d", overflow: "hidden", x: 572, y: 148, key: "c7" },
  { origin: "right bottom", flat: "flat", overflow: "visible", x: 8, y: 288, key: "c8" },
  { origin: "right bottom", flat: "flat", overflow: "hidden", x: 196, y: 288, key: "c9" },
  { origin: "right bottom", flat: "preserve-3d", overflow: "visible", x: 384, y: 288, key: "c10" },
  { origin: "right bottom", flat: "preserve-3d", overflow: "hidden", x: 572, y: 288, key: "c11" },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 436, background: "white" }}>
      {CASES.map((c) => (
        <View
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 112,
            perspective: "240px",
            perspectiveOrigin: c.origin,
            background: "#eef3f8",
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 24,
              top: 20,
              width: 112,
              height: 76,
              transform: "rotateY(35deg)",
              transformStyle: c.flat,
              overflow: c.overflow,
            }}
          >
            <View
              style={{
                position: "absolute",
                width: 76,
                height: 56,
                background: "#2463a0",
                transform: "translateZ(28px) rotateX(25deg)",
              }}
            />
            <View
              style={{
                position: "absolute",
                left: 36,
                top: 24,
                width: 76,
                height: 56,
                background: "#e0b060",
                transform: "translateZ(-16px)",
              }}
            />
          </View>
        </View>
      ))}
    </View>
  );
}
