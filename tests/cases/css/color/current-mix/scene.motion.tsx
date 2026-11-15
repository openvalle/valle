export const composition = { width: 480, height: 280, fps: 4, duration: 2 };
export const controls = {
  resources: { Sans: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }) },
};
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View
      style={{
        width: 480,
        height: 280,
        background: "white",
        color: seconds < 1 ? "#124bcc" : "#cc2612",
      }}
    >
      <View
        style={{
          position: "absolute",
          top: 20,
          width: 85,
          height: 60,
          left: 20,
          background: "color-mix(in srgb, currentColor 40%, white)",
          border: "5px solid color-mix(in srgb, currentColor, black)",
        }}
      />
      <View
        style={{
          position: "absolute",
          top: 20,
          width: 85,
          height: 60,
          left: 130,
          background:
            "linear-gradient(90deg, color-mix(in srgb, currentColor, white), color-mix(in oklab, currentColor, #22cc99))",
        }}
      />
      <View
        style={{
          position: "absolute",
          top: 20,
          width: 85,
          height: 60,
          left: 240,
          background: "currentColor",
          filter: "drop-shadow(6px 6px 2px color-mix(in srgb, currentColor, transparent))",
        }}
      />
      <View
        style={{
          position: "absolute",
          top: 20,
          width: 85,
          height: 60,
          left: 350,
          background: "color-mix(in srgb, color-mix(in srgb, currentColor, white), black)",
          outline: "3px solid color-mix(in srgb, currentColor, white)",
        }}
      />
      <Text
        style={{
          position: "absolute",
          left: 20,
          top: 100,
          fontFamily: "Sans",
          fontSize: 26,
          lineHeight: "40px",
          color: "red",
          WebkitTextFillColor: "color-mix(in srgb, currentColor, white)",
          textShadow: "1px 2px color-mix(in srgb, currentColor, black)",
        }}
      >
        Rose <Span style={{ color: "blue" }}>Blue</Span>
      </Text>
      <svg width={440} height={100} style={{ position: "absolute", left: 20, top: 165 }}>
        <defs>
          <linearGradient id="mixed">
            <stop offset={0} stopColor="color-mix(in srgb, currentColor, white)" />
            <stop offset={1} stopColor="currentColor" />
          </linearGradient>
        </defs>
        <g fill="color-mix(in srgb, currentColor, white)">
          {[0, 1, 2].map((i) => (
            <circle
              cx={30 + i * 75}
              cy={35}
              r={25}
              color={i === 0 ? "red" : i === 1 ? "blue" : "color-mix(in srgb, currentColor, white)"}
            />
          ))}
        </g>
        <rect x={245} y={10} width={80} height={60} fill="url(#mixed)" />
        <rect
          x={345}
          y={10}
          width={80}
          height={60}
          fill="color-mix(in srgb, currentColor, transparent)"
          stroke="color-mix(in srgb, currentColor, black)"
          strokeWidth={4}
        />
      </svg>
    </View>
  );
}
