export const composition = { width: 948, height: 384, fps: 1, duration: 1 };
const CASES = ["none", "blur(3px)", "drop-shadow(6px 4px 2px #702080)"].flatMap((filter) =>
  ["none", "rotate(8deg)", "scale(1.2)"].flatMap((transform) =>
    ["visible", "hidden"].map((overflow) => ({ filter, transform, overflow })),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 948, height: 384, background: "white" }}>
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
              overflow: c.overflow,
              borderRadius: 12,
            }}
          >
            <View
              style={{
                position: "absolute",
                left: -4,
                top: 12,
                width: 112,
                height: 56,
                background: "#d040a0",
                filter: c.filter,
                transform: c.transform,
              }}
            >
              <View
                style={{
                  position: "absolute",
                  left: 24,
                  top: 12,
                  width: 48,
                  height: 32,
                  background: "#40d080",
                }}
              />
            </View>
          </View>
        </View>
      ))}
    </View>
  );
}
