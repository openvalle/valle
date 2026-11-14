export const composition = { width: 240, height: 180, fps: 4, duration: 2 };

const DOTS = Array.from({ length: 9 }, (_, i) => ({ i, x: 30 + i * 22 }));

function Dots() {
  const { seconds: t } = useCurrentTime();
  return (
    <svg width={240} height={180}>
      <g fillOpacity={0.7} color="#d45d49" transform={`translate(0 ${t * 3})`}>
        {DOTS.map((dot) => (
          <circle
            key={`dot-${dot.i}`}
            cx={dot.x}
            cy={44 + Math.sin(t * 3 + dot.i) * 12}
            r={9 + 6 * Math.sin(t + dot.i) ** 2}
            fill={dot.i % 2 ? "currentColor" : "#176bba99"}
          />
        ))}
      </g>
      <g stroke="#103f5f" strokeWidth={2} strokeDasharray="3 2" opacity={0.65}>
        {DOTS.map((dot) => (
          <ellipse
            key={`oval-${dot.i}`}
            cx={dot.x}
            cy={118 + Math.cos(t * 2 + dot.i) * 8}
            rx={7 + t * 3}
            ry="8%"
            fill={dot.i % 2 ? "#efb750" : "#70bbc5"}
          />
        ))}
      </g>
    </svg>
  );
}

export default function Scene() {
  return (
    <View
      style={{
        width: 240,
        height: 180,
        background: "#f5f3ee",
        overflow: "hidden",
        borderRadius: 12,
      }}
    >
      <TimeScope offset={0.15} rate={1.2}>
        <Dots />
      </TimeScope>
    </View>
  );
}
