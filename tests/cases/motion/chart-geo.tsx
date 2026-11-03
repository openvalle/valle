export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

const square = (x, y, d) => [
  [x, y],
  [x, y + d],
  [x + d, y + d],
  [x + d, y],
  [x, y],
];

const land = {
  type: "FeatureCollection",
  features: [
    { type: "Feature", geometry: { type: "Polygon", coordinates: [square(-20, 25, 40)] } },
    { type: "Feature", geometry: { type: "Polygon", coordinates: [square(50, -25, 45)] } },
    { type: "Feature", geometry: { type: "Polygon", coordinates: [square(150, 0, 45)] } },
  ],
};

const towns = {
  type: "MultiPoint",
  coordinates: [
    [0, 45],
    [70, -5],
    [170, 20],
  ],
};

const route = {
  type: "LineString",
  coordinates: [
    [0, 45],
    [40, 30],
    [70, -5],
  ],
};

// d3's maps: a Mercator projection fitted to two of the lands and cut to its frame, with towns
// that grow and a route, and the whole world turning with the time, the land across the edge
// cut in two.
export default function Scene() {
  const { progress } = useCurrentTime();
  const map = geoMercator()
    .fitExtent(
      [
        [8, 8],
        [168, 152],
      ],
      { type: "FeatureCollection", features: land.features.slice(0, 2) },
    )
    .clipExtent([
      [0, 0],
      [176, 160],
    ]);
  const path = geoPath(map);
  const turning = geoMercator()
    .rotate([-180 * progress, 0])
    .scale(36 / Math.PI)
    .translate([36, 36])
    .clipExtent([
      [0, 0],
      [72, 72],
    ]);
  const small = geoPath(turning);
  return (
    <svg width={256} height={160}>
      <rect width={176} height={160} fill="#e3f2fd" />
      <path d={path(land)} fill="#81c784" stroke="#2e7d32" strokeWidth={1} />
      <path d={path(route)} fill="none" stroke="#e53935" strokeWidth={2} />
      <path d={path.pointRadius(2 + 3 * progress)(towns)} fill="#37474f" />
      <g transform="translate(176 44)">
        <rect width={72} height={72} fill="#e3f2fd" />
        <path d={small(land)} fill="#ffb74d" stroke="#e65100" strokeWidth={1} />
      </g>
    </svg>
  );
}
