export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

const months = [
  { apples: 10, pears: 5, plums: 2 },
  { apples: 12, pears: 6, plums: 4 },
  { apples: 9, pears: 9, plums: 6 },
  { apples: 14, pears: 7, plums: 5 },
  { apples: 16, pears: 10, plums: 8 },
  { apples: 13, pears: 12, plums: 9 },
];
const colors = { apples: "#e53935", pears: "#7cb342", plums: "#5e35b1" };

// A stacked area chart of d3's stack and area, the layers rising with the time.
export default function Scene() {
  const { progress } = useCurrentTime();
  const layers = stack()
    .keys(["apples", "pears", "plums"])
    .value((d, key) => d[key] * progress)(months);
  const x = scaleLinear()
    .domain([0, months.length - 1])
    .range([8, 248]);
  const y = scaleLinear().domain([0, 32]).range([152, 8]);
  const shape = area()
    .x((_, i) => x(i))
    .y0((p) => y(p[0]))
    .y1((p) => y(p[1]))
    .curve(curveMonotoneX);
  return (
    <svg width={256} height={160}>
      {layers.map((layer) => (
        <path key={layer.key} d={shape(layer)} fill={colors[layer.key]} />
      ))}
    </svg>
  );
}
