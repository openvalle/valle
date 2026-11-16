export const composition = { width: 948, height: 756, fps: 1, duration: 1 };
const CASES = ["none", "invert(1)", "brightness(0.5)", "blur(3px)"].flatMap((filter) =>
  ["normal", "multiply", "screen"].flatMap((blend) =>
    [0, 0.5, 1].map((opacity) => ({ filter, blend, opacity })),
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
              background: "rgb(128 192 64 / 50%)",
              backdropFilter: c.filter,
              mixBlendMode: c.blend,
              opacity: c.opacity,
            }}
          >
            <View
              style={{
                position: "absolute",
                left: 24,
                top: 16,
                width: 48,
                height: 32,
                background: "#d040a0",
              }}
            />
          </View>
        </View>
      ))}
    </View>
  );
}
