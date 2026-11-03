export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

const shares = [
  { name: "a", value: 4, color: "#1e88e5" },
  { name: "b", value: 3, color: "#43a047" },
  { name: "c", value: 2, color: "#fb8c00" },
  { name: "d", value: 1, color: "#8e24aa" },
];

// A donut of d3's pie and arc with rounded, padded slices sweeping in with the time, and a
// progress ring filling up.
export default function Scene() {
  const { progress } = useCurrentTime();
  const slices = pie()
    .value((d) => d.value)
    .sort(null)
    .padAngle(0.04)
    .endAngle(2 * Math.PI * progress + 0.2)(shares);
  const donut = arc().innerRadius(36).outerRadius(64).cornerRadius(6);
  const ring = arc()
    .innerRadius(40)
    .outerRadius(52)
    .cornerRadius(6)
    .startAngle(0)
    .endAngle(2 * Math.PI * progress + 0.3);
  return (
    <svg width={256} height={160}>
      <g transform="translate(80 80)">
        {slices.map((s) => (
          <path key={s.data.name} d={donut(s)} fill={s.data.color} />
        ))}
      </g>
      <g transform="translate(192 80)">
        <circle r={46} fill="none" stroke="#eceff1" strokeWidth={12} />
        <path d={ring()} fill="#e53935" />
      </g>
    </svg>
  );
}
