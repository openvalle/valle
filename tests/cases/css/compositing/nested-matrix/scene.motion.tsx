export const composition = { width: 948, height: 756, fps: 1, duration: 1 };
const CASES = ["auto", "isolate"].flatMap((isolation) =>
  [0.5, 1].flatMap((opacity) =>
    ["visible", "hidden"].flatMap((overflow) =>
      [false, true].flatMap((content) =>
        ["normal", "multiply"].map((blend) => ({ isolation, opacity, overflow, content, blend })),
      ),
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
              isolation: c.isolation,
              opacity: c.opacity,
              overflow: c.overflow,
            }}
          >
            <View
              style={{
                position: "absolute",
                left: -8,
                top: 12,
                width: 120,
                height: 56,
                backdropFilter: "invert(1)",
                mixBlendMode: c.blend,
              }}
            >
              {c.content && (
                <View
                  style={{
                    position: "absolute",
                    left: 36,
                    top: 8,
                    width: 76,
                    height: 56,
                    background: "#40d080",
                  }}
                />
              )}
            </View>
          </View>
        </View>
      ))}
    </View>
  );
}
