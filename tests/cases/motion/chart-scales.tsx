export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

const rows = [
  { name: "a", value: 30 },
  { name: "b", value: 80 },
  { name: "c", value: 55 },
  { name: "d", value: 95 },
  { name: "e", value: 20 },
];

// Bars growing with the time through d3's scales: a band scale places them, a linear one sizes
// them and blends their colors, a point scale puts a dot over each, and the ticks of the linear
// scale draw the grid.
export default function Scene() {
  const { progress } = useCurrentTime();
  const x = scaleBand()
    .domain(rows.map((r) => r.name))
    .range([24, 248])
    .padding(0.25);
  const dots = scalePoint()
    .domain(rows.map((r) => r.name))
    .range([24, 248])
    .padding(0.5);
  const y = scaleLinear().domain([0, 100]).range([144, 16]).nice();
  const color = scaleLinear().domain([0, 100]).range(["#90caf9", "#e53935"]);
  return (
    <svg width={256} height={160}>
      {y.ticks(4).map((t) => (
        <line key={t} x1={20} x2={252} y1={y(t)} y2={y(t)} stroke="#cfd8dc" strokeWidth={1} />
      ))}
      {rows.map((r) => {
        const v = r.value * progress;
        return (
          <rect
            key={r.name}
            x={x(r.name)}
            y={y(v)}
            width={x.bandwidth()}
            height={y(0) - y(v)}
            fill={color(v)}
          />
        );
      })}
      {rows.map((r) => (
        <circle
          key={r.name}
          cx={dots(r.name)}
          cy={y(r.value * progress) - 8}
          r={3}
          fill="#37474f"
        />
      ))}
    </svg>
  );
}
