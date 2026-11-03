export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

const steps = ["idea", "draft", "review", "fix", "ship", "notes"];
// The fix goes back to review: a cycle, laid out as a back edge.
const links = [
  [0, 1],
  [1, 2],
  [2, 3],
  [3, 2],
  [2, 4],
  [0, 5],
];

// A flow chart laid out in ranks by graphLayout, its edges drawn in with the time, and the same
// graph flowing right beside it, each centered by its width and height.
export default function Scene() {
  const { progress } = useCurrentTime();
  const down = graphLayout({
    sizes: steps.map(() => [36, 14]),
    edges: links,
    nodeGap: 12,
    rankGap: 18,
  });
  const right = graphLayout({
    sizes: steps.map(() => [10, 6]),
    edges: links,
    direction: "right",
    nodeGap: 4,
    rankGap: 8,
  });
  return (
    <svg width={256} height={160}>
      <g transform={`translate(${(176 - down.width) / 2} ${(160 - down.height) / 2})`}>
        {links.map(([a, b], i) => {
          const [x1, y1] = down.centers[a];
          const [x2, y2] = down.centers[b];
          const bend = down.backEdges[i] ? 28 : 0;
          const d = `M${x1},${y1} C${x1 + bend},${y1} ${x2 + bend},${y2} ${x2},${y2}`;
          const length = pathLength(d);
          return (
            <path
              key={i}
              d={d}
              fill="none"
              stroke={down.backEdges[i] ? "#e53935" : "#90a4ae"}
              strokeWidth={2}
              strokeDasharray={`${length} ${length}`}
              strokeDashoffset={length * (1 - progress)}
            />
          );
        })}
        {down.centers.map(([x, y], i) => (
          <rect key={i} x={x - 18} y={y - 7} width={36} height={14} rx={4} fill="#1e88e5" />
        ))}
      </g>
      <g transform={`translate(${216 - right.width / 2} ${80 - right.height / 2})`}>
        {links.map(([a, b], i) => (
          <line
            key={i}
            x1={right.centers[a][0]}
            y1={right.centers[a][1]}
            x2={right.centers[b][0]}
            y2={right.centers[b][1]}
            stroke="#90a4ae"
            strokeWidth={1}
          />
        ))}
        {right.centers.map(([x, y], i) => (
          <rect key={i} x={x - 5} y={y - 3} width={10} height={6} fill="#43a047" />
        ))}
      </g>
    </svg>
  );
}
