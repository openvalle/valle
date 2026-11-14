export const composition = { width: 240, height: 180, fps: 4, duration: 2 };

const ROWS = Array.from({ length: 7 }, (_, i) => ({ i, x: 20 + i * 30 }));

export default function Scene() {
  const { seconds: t } = useCurrentTime();
  return (
    <View style={{ background: "#f5f3ee", overflow: "hidden", borderRadius: 12 }}>
      <svg width={240} height={180}>
        <g color="#d45d49" opacity={0.8} transform={`translate(0 ${t * 3})`}>
          {ROWS.map((row) => (
            <rect
              key={`bar-${row.i}`}
              x={row.x}
              y={70 - 28 * Math.sin(t * 2 + row.i)}
              width={20}
              height={30 + 24 * Math.sin(t * 2 + row.i)}
              rx="auto"
              ry="3%"
              fill={row.i % 2 ? "currentColor" : "#176bba99"}
              stroke={row.i % 2 ? "#103f5f" : "none"}
              strokeWidth={2}
              strokeOpacity={0.6}
            />
          ))}
        </g>
        <g color="#103f5f" strokeOpacity={0.65}>
          {ROWS.map((row) => (
            <line
              key={`tick-${row.i}`}
              x1={row.x}
              y1="70%"
              x2={row.x + 20}
              y2={146 + 10 * Math.sin(t * 2 + row.i)}
              stroke={row.i % 2 ? "currentColor" : "#d45d49"}
              strokeWidth={3}
              strokeLinecap="round"
              strokeDasharray="3 2"
              pathLength={20}
              transform="translate(0 -2)"
            />
          ))}
        </g>
      </svg>
    </View>
  );
}
