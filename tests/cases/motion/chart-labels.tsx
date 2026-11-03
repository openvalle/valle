export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

const dots = [
  { at: [60, 50], color: "#1e88e5", priority: 3 },
  { at: [80, 60], color: "#43a047", priority: 2 },
  { at: [70, 80], color: "#fb8c00", priority: 1 },
  { at: [180, 40], color: "#8e24aa", priority: 0 },
  { at: [236, 140], color: "#e53935", priority: 0 },
];

// Labels kept off each other and inside the frame as their dots drift with the time: each takes
// the first free place around its dot, by priority, and one with none is hidden.
export default function Scene() {
  const { progress } = useCurrentTime();
  const points = dots.map((d, i) => [d.at[0] + 24 * progress * (i % 2 ? 1 : -1), d.at[1]]);
  const labels = placeLabels(
    points.map((p, i) => ({ point: p, size: [36, 12], priority: dots[i].priority })),
    { bounds: [0, 0, 256, 160], padding: 3 },
  );
  return (
    <svg width={256} height={160}>
      <rect x={1} y={1} width={254} height={158} fill="none" stroke="#cfd8dc" strokeWidth={2} />
      {dots.map((d, i) => (
        <circle key={i} cx={points[i][0]} cy={points[i][1]} r={3} fill={d.color} />
      ))}
      {labels.map((l, i) => (
        <rect
          key={i}
          x={l.x}
          y={l.y}
          width={36}
          height={12}
          rx={3}
          fill={dots[i].color}
          opacity={l.visible ? 0.8 : 0}
        />
      ))}
    </svg>
  );
}
