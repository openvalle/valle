export const composition = { width: 1520, height: 480, fps: 1, duration: 1 };

const CASES = [
  { blend: "normal", filter: "none", x: 8, y: 8, key: "c0" },
  { blend: "normal", filter: "invert(1)", x: 196, y: 8, key: "c1" },
  { blend: "multiply", filter: "none", x: 384, y: 8, key: "c2" },
  { blend: "multiply", filter: "invert(1)", x: 572, y: 8, key: "c3" },
  { blend: "screen", filter: "none", x: 760, y: 8, key: "c4" },
  { blend: "screen", filter: "invert(1)", x: 948, y: 8, key: "c5" },
  { blend: "overlay", filter: "none", x: 1136, y: 8, key: "c6" },
  { blend: "overlay", filter: "invert(1)", x: 1324, y: 8, key: "c7" },
  { blend: "darken", filter: "none", x: 8, y: 124, key: "c8" },
  { blend: "darken", filter: "invert(1)", x: 196, y: 124, key: "c9" },
  { blend: "lighten", filter: "none", x: 384, y: 124, key: "c10" },
  { blend: "lighten", filter: "invert(1)", x: 572, y: 124, key: "c11" },
  { blend: "color-dodge", filter: "none", x: 760, y: 124, key: "c12" },
  { blend: "color-dodge", filter: "invert(1)", x: 948, y: 124, key: "c13" },
  { blend: "color-burn", filter: "none", x: 1136, y: 124, key: "c14" },
  { blend: "color-burn", filter: "invert(1)", x: 1324, y: 124, key: "c15" },
  { blend: "hard-light", filter: "none", x: 8, y: 240, key: "c16" },
  { blend: "hard-light", filter: "invert(1)", x: 196, y: 240, key: "c17" },
  { blend: "soft-light", filter: "none", x: 384, y: 240, key: "c18" },
  { blend: "soft-light", filter: "invert(1)", x: 572, y: 240, key: "c19" },
  { blend: "difference", filter: "none", x: 760, y: 240, key: "c20" },
  { blend: "difference", filter: "invert(1)", x: 948, y: 240, key: "c21" },
  { blend: "exclusion", filter: "none", x: 1136, y: 240, key: "c22" },
  { blend: "exclusion", filter: "invert(1)", x: 1324, y: 240, key: "c23" },
  { blend: "hue", filter: "none", x: 8, y: 356, key: "c24" },
  { blend: "hue", filter: "invert(1)", x: 196, y: 356, key: "c25" },
  { blend: "saturation", filter: "none", x: 384, y: 356, key: "c26" },
  { blend: "saturation", filter: "invert(1)", x: 572, y: 356, key: "c27" },
  { blend: "color", filter: "none", x: 760, y: 356, key: "c28" },
  { blend: "color", filter: "invert(1)", x: 948, y: 356, key: "c29" },
  { blend: "luminosity", filter: "none", x: 1136, y: 356, key: "c30" },
  { blend: "luminosity", filter: "invert(1)", x: 1324, y: 356, key: "c31" },
];
export default function Scene() {
  return (
    <View style={{ width: 1520, height: 480, background: "white" }}>
      {CASES.map((c) => (
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
            style={{
              position: "absolute",
              left: 12,
              top: 12,
              width: 132,
              height: 68,
              background: "#c05080",
              mixBlendMode: c.blend,
              backdropFilter: c.filter,
            }}
          >
            <View style={{ width: 52, height: 28, background: "#40a080", opacity: 0.5 }} />
          </View>
        </View>
      ))}
    </View>
  );
}
