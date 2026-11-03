export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

const days = [12, 30, 22, 48, 41, 63, 57, 80, 72, 90];

// A line and the area under it growing with the time: d3's line and area with the monotone
// curve, over values that depend on the time, and a band around a second series.
export default function Scene() {
  const { progress } = useCurrentTime();
  const x = scaleLinear()
    .domain([0, days.length - 1])
    .range([16, 240]);
  const y = scaleLinear().domain([0, 100]).range([144, 16]);
  const grow = (v) => y(v * progress);
  const curve = line()
    .x((_, i) => x(i))
    .y((v) => grow(v))
    .curve(curveMonotoneX);
  const under = area()
    .x((_, i) => x(i))
    .y0(y(0))
    .y1((v) => grow(v))
    .curve(curveMonotoneX);
  const band = area()
    .x((_, i) => x(i))
    .y0((v) => y(100 - v) + 6)
    .y1((v) => y(100 - v) - 6);
  return (
    <svg width={256} height={160}>
      <path d={band(days)} fill="#ffe0b2" />
      <path d={under(days)} fill="#bbdefb" />
      <path d={curve(days)} fill="none" stroke="#1565c0" strokeWidth={2} />
    </svg>
  );
}
