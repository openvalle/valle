export const composition = { width: 240, height: 180, fps: 4, duration: 2 };

const ROWS = Array.from({ length: 5 }, (_, index) => ({
  key: `row-${index}`,
  index,
  y: 30 + index * 28,
}));
const POINTS = [0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1];

function envelope(u) {
  return 0.4 + Math.sin(u * Math.PI);
}

export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 240, height: 180, background: "#f5f3ee" }}>
      <svg key="lines" width={240} height={180}>
        <defs>
          <linearGradient id="ink">
            <stop stopColor="#176bba" />
            <stop offset="1" stopColor="#e66a38" />
          </linearGradient>
          <marker id="tip" markerWidth={4} markerHeight={4} refX={2} refY={2} orient="auto">
            <polygon points="0,0 4,2 0,4" fill="#e66a38" />
          </marker>
          <clipPath id="clip">
            <rect x={10} y={8} width={220} height={164} rx={8} />
          </clipPath>
        </defs>
        <g clipPath="url(#clip)" strokeLinejoin="round">
          {ROWS.map((row) => {
            const rise = interpolate(seconds, [row.index * 0.06, 1 + row.index * 0.06], [0, 1], {
              easing: "easeOutCubic",
            });
            let d = "";
            for (const u of POINTS) {
              const n = noise2d(7, u * 4 + row.index * 0.3, seconds * 0.7);
              const y = row.y - rise * envelope(u) * n * 18;
              d += `${u === 0 ? "M" : "L"}${18 + u * 204},${y}`;
            }
            return (
              <path
                key={row.key}
                d={d}
                fill="none"
                stroke="url(#ink)"
                strokeWidth={2}
                strokeDasharray="6 2"
                markerEnd="url(#tip)"
              />
            );
          })}
          {[0, 1, 2, 3, 4].map((i) => (
            <circle
              key={`dot-${i}`}
              cx={18 + i * 51}
              cy={164}
              r={2 + 2 * Math.sin(seconds + i) ** 2}
              fill="#176bba"
            />
          ))}
        </g>
      </svg>
    </View>
  );
}
