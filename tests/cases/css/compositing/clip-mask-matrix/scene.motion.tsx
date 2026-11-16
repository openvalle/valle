export const composition = { width: 948, height: 756, fps: 1, duration: 1 };
const CASES = [
  "none",
  "linear-gradient(to right, black 50%, transparent 50%)",
  "linear-gradient(to right, black, transparent)",
].flatMap((mask) =>
  ["none", "inset(6px)", "circle(42%)"].flatMap((clip) =>
    ["none", "rotate(8deg)"].flatMap((transform) =>
      [0.5, 1].map((opacity) => ({ mask, clip, transform, opacity })),
    ),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 948, height: 756, background: "white" }}>
      {CASES.map((c, i) => (
        <View
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 6) * 156,
            top: 12 + Math.floor(i / 6) * 124,
            width: 140,
            height: 108,
            background: "#4080c0",
          }}
        >
          {[0, 1, 2, 3, 4].map((j) => (
            <View
              style={{
                position: "absolute",
                left: j * 28,
                width: 14,
                height: 108,
                background: "#e0b060",
              }}
            />
          ))}
          <View
            style={{
              position: "absolute",
              left: 16,
              top: 16,
              width: 108,
              height: 76,
              background: "#d040a0",
              borderRadius: 12,
              maskImage: c.mask,
              clipPath: c.clip,
              transform: c.transform,
              opacity: c.opacity,
            }}
          >
            <View
              style={{
                position: "absolute",
                left: -10,
                top: 24,
                width: 128,
                height: 24,
                background: "#40d080",
              }}
            />
          </View>
        </View>
      ))}
    </View>
  );
}
