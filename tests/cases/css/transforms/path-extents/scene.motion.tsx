export const composition = { width: 768, height: 576, fps: 1, duration: 1 };

const CASES = [
  { distance: "-25%", anchor: "center", rotate: "auto", x: 8, y: 8, key: "c0" },
  { distance: "-25%", anchor: "center", rotate: "reverse", x: 196, y: 8, key: "c1" },
  { distance: "-25%", anchor: "left top", rotate: "auto", x: 384, y: 8, key: "c2" },
  { distance: "-25%", anchor: "left top", rotate: "reverse", x: 572, y: 8, key: "c3" },
  { distance: "0%", anchor: "center", rotate: "auto", x: 8, y: 148, key: "c4" },
  { distance: "0%", anchor: "center", rotate: "reverse", x: 196, y: 148, key: "c5" },
  { distance: "0%", anchor: "left top", rotate: "auto", x: 384, y: 148, key: "c6" },
  { distance: "0%", anchor: "left top", rotate: "reverse", x: 572, y: 148, key: "c7" },
  { distance: "50%", anchor: "center", rotate: "auto", x: 8, y: 288, key: "c8" },
  { distance: "50%", anchor: "center", rotate: "reverse", x: 196, y: 288, key: "c9" },
  { distance: "50%", anchor: "left top", rotate: "auto", x: 384, y: 288, key: "c10" },
  { distance: "50%", anchor: "left top", rotate: "reverse", x: 572, y: 288, key: "c11" },
  { distance: "125%", anchor: "center", rotate: "auto", x: 8, y: 428, key: "c12" },
  { distance: "125%", anchor: "center", rotate: "reverse", x: 196, y: 428, key: "c13" },
  { distance: "125%", anchor: "left top", rotate: "auto", x: 384, y: 428, key: "c14" },
  { distance: "125%", anchor: "left top", rotate: "reverse", x: 572, y: 428, key: "c15" },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 576, background: "white" }}>
      {CASES.map((c) => (
        <View
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 112,
            background: "#eef3f8",
          }}
        >
          <View
            style={{
              position: "absolute",
              width: 28,
              height: 16,
              background: "#2463a0",
              offsetPath: 'path("M 24 64 Q 78 0 132 64")',
              offsetDistance: c.distance,
              offsetAnchor: c.anchor,
              offsetRotate: c.rotate,
            }}
          >
            <View style={{ width: 8, height: 8, background: "#e0b060" }} />
          </View>
        </View>
      ))}
    </View>
  );
}
