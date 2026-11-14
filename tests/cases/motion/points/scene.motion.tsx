function formatted(points) {
  let s = "";
  for (const p of points) s += `${p} `;
  return s;
}
export const composition = { width: 240, height: 160, fps: 4, duration: 2 };

export default function Scene() {
  const { seconds } = useCurrentTime();
  const measured = bounds("measure");
  const points = [];
  for (let i = 0; i < 9; i++) {
    points.push(20 + i * 25, 40 + 20 * Math.sin(seconds * 2 + i * 0.7));
  }
  const triangle = seconds < 1 ? [20, 140, 60, 90, 100, 140] : [30, 140, 60, 110, 90, 140];
  return (
    <View style={{ width: 240, height: 160, background: "white" }}>
      <View key="measure" style={{ position: "absolute", width: 130 + seconds * 30, height: 1 }} />
      <svg width={240} height={160}>
        <defs>
          <linearGradient id="ink">
            <stop stopColor="#176bba" />
            <stop offset="1" stopColor="#e66a38" />
          </linearGradient>
          <marker id="tip" markerWidth={4} markerHeight={4} refX={2} refY={2} orient="auto">
            <polygon points={formatted([0, 0, 4, 2, 0, 4])} fill="#e66a38" />
          </marker>
          <clipPath id="clip">
            <rect width={230} height={155} />
          </clipPath>
        </defs>
        <g clipPath="url(#clip)">
          <polyline
            points={formatted(points)}
            fill="none"
            stroke="url(#ink)"
            strokeWidth={3}
            strokeLinejoin="round"
            strokeDasharray="8 3"
            markerEnd="url(#tip)"
          />
          <polygon points={formatted(triangle)} fill="#176bba" stroke="#122c46" strokeWidth={2} />
          <path
            d={`M120,140L${measured.width},95L220,140`}
            fill="none"
            stroke="#e66a38"
            strokeWidth={4}
            strokeLinejoin="round"
          />
          <polyline points={formatted([])} stroke="red" />
          <polygon points={formatted([230, 150])} fill="red" />
        </g>
      </svg>
    </View>
  );
}
