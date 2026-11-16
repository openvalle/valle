export const composition = { width: 948, height: 136, fps: 1, duration: 1 };
const CASES = [0, 1, 2, 3, 4, 5];
export default function Scene() {
  return (
    <View style={{ width: 948, height: 136, background: "white" }}>
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
              backdropFilter: "invert(1)",
              mixBlendMode: c === 5 ? "multiply" : "normal",
              opacity: c === 1 ? 0 : 1,
              visibility: c === 2 || c >= 4 ? "hidden" : "visible",
              display: c === 3 ? "none" : "block",
            }}
          >
            <View
              style={{
                position: "absolute",
                left: 24,
                top: 16,
                width: 48,
                height: 32,
                background: "#40d080",
                visibility: c >= 4 ? "visible" : "inherit",
              }}
            />
          </View>
        </View>
      ))}
    </View>
  );
}
