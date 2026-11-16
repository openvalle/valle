export const composition = { width: 652, height: 344, fps: 1, duration: 1 };
const OVERFLOW = [76, 120].flatMap((width) =>
  ["clip", "ellipsis"].flatMap((overflow) =>
    [false, true].map((rtl) => ({ width, overflow, rtl })),
  ),
);
const CLAMP = [16, 24].flatMap((size) => [1, 2].map((count) => ({ size, count })));
export default function Scene() {
  return (
    <View style={{ width: 652, height: 344, background: "white" }}>
      {OVERFLOW.map((c, i) => (
        <Text
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 4) * 160,
            top: 12 + Math.floor(i / 4) * 112,
            width: c.width,
            height: 96,
            fontSize: 24,
            lineHeight: "32px",
            color: "#17365d",
            background: "#eef3f8",
            overflow: "hidden",
            whiteSpace: "nowrap",
            textOverflow: c.overflow,
            direction: c.rtl ? "rtl" : "ltr",
          }}
        >
          ALPHA BETA GAMMA
        </Text>
      ))}
      {CLAMP.map((c, i) => (
        <Text
          key={`c${8 + i}`}
          style={{
            position: "absolute",
            left: 12 + i * 160,
            top: 236,
            width: 136,
            fontSize: c.size,
            lineHeight: "32px",
            color: "#17365d",
            background: "#eef3f8",
            overflow: "hidden",
            display: "-webkit-box",
            WebkitBoxOrient: "vertical",
            WebkitLineClamp: c.count,
          }}
        >
          ALPHA BETA GAMMA DELTA EPSILON
        </Text>
      ))}
    </View>
  );
}
