export const composition = { width: 420, height: 300, fps: 1, duration: 1 };

// Charts as data drawn with SVG: bars of a list over dashed grid lines; a line with an area under
// it, a gradient in user units, and dots; a donut of dashes by path length, turned to start at the
// top; a pie of arcs; stacked bars with rounded ends clipped to the plot; and a sparkline.
export default function Scene() {
  const data = [12, 34, 26, 48, 40, 62, 54, 76];
  const colors = ["#1e88e5", "#43a047", "#fb8c00", "#e53935"];
  const shares = [40, 30, 20, 10];
  const x = (i) => 10 + i * 26;
  const y = (v) => 90 - v;
  const line = data.map((v, i) => `${x(i)},${y(v)}`).join(" ");
  const area = `M${x(0)} 90 L${line} L${x(7)} 90 Z`;
  let turned = 0;
  const donut = shares.map((share, i) => {
    const offset = -turned;
    turned += share;
    return (
      <circle
        key={i}
        cx={50}
        cy={50}
        r={36}
        fill="none"
        stroke={colors[i]}
        strokeWidth={16}
        pathLength={100}
        strokeDasharray={`${share} ${100 - share}`}
        strokeDashoffset={offset}
      />
    );
  });
  const point = (percent) => {
    const a = (percent / 100) * 2 * Math.PI - Math.PI / 2;
    return `${50 + 42 * Math.cos(a)} ${50 + 42 * Math.sin(a)}`;
  };
  let reached = 0;
  const pie = shares.map((share, i) => {
    const from = point(reached);
    reached += share;
    const large = share > 50 ? 1 : 0;
    return (
      <path
        key={i}
        d={`M50 50 L${from} A42 42 0 ${large} 1 ${point(reached)} Z`}
        fill={colors[i]}
        stroke="#fff"
        strokeWidth={1.5}
      />
    );
  });
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 12, padding: 10 }}>
      <svg width={220} height={100} style={{ background: "#fafafa" }}>
        {[20, 40, 60, 80].map((v) => (
          <line
            key={v}
            x1={0}
            x2={220}
            y1={v}
            y2={v}
            stroke="#b0bec5"
            strokeDasharray="4 3"
            strokeWidth={1}
          />
        ))}
        {data.map((v, i) => (
          <rect
            key={i}
            x={x(i) - 8}
            y={y(v)}
            width={16}
            height={v}
            rx={2}
            fill={colors[i % 4]}
            fillOpacity={0.85}
          />
        ))}
      </svg>
      <svg width={100} height={100} viewBox="0 0 100 100">
        <g transform="rotate(-90 50 50)">{donut}</g>
      </svg>
      <svg width={220} height={100} style={{ background: "#fafafa" }}>
        <defs>
          <linearGradient id="under" gradientUnits="userSpaceOnUse" x1={0} y1={14} x2={0} y2={90}>
            <stop offset={0} stopColor="#1e88e5" stopOpacity={0.6} />
            <stop offset={1} stopColor="#1e88e5" stopOpacity={0} />
          </linearGradient>
        </defs>
        <path d={area} fill="url(#under)" />
        <polyline
          points={line}
          fill="none"
          stroke="#1565c0"
          strokeWidth={2}
          strokeLinejoin="round"
        />
        {data.map((v, i) => (
          <circle
            key={i}
            cx={x(i)}
            cy={y(v)}
            r={3}
            fill="#fff"
            stroke="#1565c0"
            strokeWidth={1.5}
          />
        ))}
      </svg>
      <svg width={100} height={100} viewBox="0 0 100 100">
        {pie}
      </svg>
      <svg width={220} height={60} style={{ background: "#fafafa" }}>
        <clipPath id="plot">
          <rect x={4} y={4} width={212} height={52} rx={6} />
        </clipPath>
        <g clipPath="url(#plot)">
          {[0, 1, 2].map((row) => {
            let at = 0;
            return shares.map((share, i) => {
              const width = (share + row * 2 * (i - 1)) * 2.12;
              const rect = (
                <rect
                  key={`${row}-${i}`}
                  x={4 + at}
                  y={6 + row * 17}
                  width={width}
                  height={14}
                  fill={colors[i]}
                />
              );
              at += width;
              return rect;
            });
          })}
        </g>
      </svg>
      <svg width={100} height={40} viewBox="0 0 80 20" preserveAspectRatio="none">
        <polyline
          points={data.map((v, i) => `${i * 11},${20 - v / 4}`).join(" ")}
          fill="none"
          stroke="#00897b"
          strokeWidth={1}
        />
      </svg>
    </View>
  );
}
