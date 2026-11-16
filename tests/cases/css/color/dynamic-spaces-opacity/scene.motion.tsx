export const composition = { width: 204, height: 132, fps: 4, duration: 2 };

const CASES = [
  { color: "color(srgb 0.25 0.5 0.75)", opacity: 0.5, x: 8, y: 8, key: "c0" },
  { color: "color(srgb 0.25 0.5 0.75)", opacity: 1, x: 196, y: 8, key: "c1" },
  { color: "color(srgb-linear 0.05 0.2 0.5)", opacity: 0.5, x: 384, y: 8, key: "c2" },
  { color: "color(srgb-linear 0.05 0.2 0.5)", opacity: 1, x: 572, y: 8, key: "c3" },
  { color: "color(display-p3 0.2 0.6 0.4)", opacity: 0.5, x: 8, y: 124, key: "c4" },
  { color: "color(display-p3 0.2 0.6 0.4)", opacity: 1, x: 196, y: 124, key: "c5" },
  { color: "color(a98-rgb 0.3 0.5 0.7)", opacity: 0.5, x: 384, y: 124, key: "c6" },
  { color: "color(a98-rgb 0.3 0.5 0.7)", opacity: 1, x: 572, y: 124, key: "c7" },
  { color: "color(prophoto-rgb 0.2 0.3 0.4)", opacity: 0.5, x: 8, y: 240, key: "c8" },
  { color: "color(prophoto-rgb 0.2 0.3 0.4)", opacity: 1, x: 196, y: 240, key: "c9" },
  { color: "color(rec2020 0.2 0.4 0.6)", opacity: 0.5, x: 384, y: 240, key: "c10" },
  { color: "color(rec2020 0.2 0.4 0.6)", opacity: 1, x: 572, y: 240, key: "c11" },
  { color: "color(xyz-d65 0.2 0.3 0.4)", opacity: 0.5, x: 8, y: 356, key: "c12" },
  { color: "color(xyz-d65 0.2 0.3 0.4)", opacity: 1, x: 196, y: 356, key: "c13" },
  { color: "color(xyz-d50 0.2 0.3 0.4)", opacity: 0.5, x: 384, y: 356, key: "c14" },
  { color: "color(xyz-d50 0.2 0.3 0.4)", opacity: 1, x: 572, y: 356, key: "c15" },
  { color: "lab(50% 20 -30)", opacity: 0.5, x: 8, y: 472, key: "c16" },
  { color: "lab(50% 20 -30)", opacity: 1, x: 196, y: 472, key: "c17" },
  { color: "lch(60% 30 120)", opacity: 0.5, x: 384, y: 472, key: "c18" },
  { color: "lch(60% 30 120)", opacity: 1, x: 572, y: 472, key: "c19" },
  { color: "oklab(60% 0.1 -0.1)", opacity: 0.5, x: 8, y: 588, key: "c20" },
  { color: "oklab(60% 0.1 -0.1)", opacity: 1, x: 196, y: 588, key: "c21" },
  { color: "oklch(70% 0.12 220)", opacity: 0.5, x: 384, y: 588, key: "c22" },
  { color: "oklch(70% 0.12 220)", opacity: 1, x: 572, y: 588, key: "c23" },
];
const STATES = [0, 1, 8, 9, 22, 23, 0, 23];
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
            width: 156,
            height: 76,
            background: "#e0b060",
          }}
        >
          <View style={{ width: 120, height: 60, background: c.color, opacity: c.opacity }} />
        </View>
      ))}
    </View>
  );
}
