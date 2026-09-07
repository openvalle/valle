// Map fixture.
//
// Use synthetic polygons and sites to test projection, fitting and viewport scaling.
//
// Derive all geometry from one geoProject call.
//
// Project once in fixed canvas coordinates. Draw each polygon edge with a fixed two-point line to preserve frame-time topology.
export const component = "world-map";

// Synthetic polygons and sites in longitude/latitude coordinates.
const LAND_A = [
  [-40, 20], [-10, 34], [8, 24], [2, -2], [-24, -12], [-44, 2],
];
const LAND_B = [
  [30, 44], [72, 52], [96, 40], [88, 12], [52, 6], [28, 20],
];
const LAND_C = [
  [110, -8], [140, -4], [150, -28], [128, -38], [108, -26],
];
const SITES = [
  { at: [-24, 12], label: "west" },
  { at: [52, 34], label: "central" },
  { at: [88, 26], label: "east" },
  { at: [130, -20], label: "south" },
];

// Project all points together so they share projection parameters and fitting bounds.
const ALL = [...LAND_A, ...LAND_B, ...LAND_C, ...SITES.map((s) => s.at)];

const CANVAS_W = 1920;
const CANVAS_H = 1080;
const P = geoProject({
  points: ALL,
  projection: "mercator",
  width: CANVAS_W,
  height: CANVAS_H,
  padding: 0.06,
});

// Split the flat projected array into polygon edges.
const A_END = LAND_A.length;
const B_END = A_END + LAND_B.length;
const C_END = B_END + LAND_C.length;
const ringEdges = (from, to) =>
  P.slice(from, to).map((p, i) => [p, P[from + ((i + 1) % (to - from))]]);
const EDGES = [
  ...ringEdges(0, A_END),
  ...ringEdges(A_END, B_END),
  ...ringEdges(B_END, C_END),
];
const SITE_XY = SITES.map((s, i) => P[C_END + i]);
// Routes originate at site zero.
const LINKS = SITE_XY.slice(1).map((xy) => [SITE_XY[0], xy]);

const DOT = CANVAS_H * 0.012;
const NAME_FONT = CANVAS_H * 0.022;

export default function WorldMap(ctx) {
  const sx = ctx.viewport.width / CANVAS_W;
  const sy = ctx.viewport.height / CANVAS_H;

  // Prepare staggered site entrances.
  const pop = (i) => interpolate(ctx.enter.progress, [0.2 + i * 0.12, 0.6 + i * 0.12], [0, 1]);

  return (
    <Scene className="h-full w-full bg-slate-950">
      {/* Fixed coastline topology; scale coordinates at frame time. */}
      {EDGES.map((e, i) => (
        <Path
          key={`coast-${i}`}
          style={{ position: "absolute", left: 0, top: 0 }}
          fill="none"
          stroke="#334155"
          strokeWidth="3"
          strokeLinecap="round"
          d={line([point(e[0][0] * sx, e[0][1] * sy), point(e[1][0] * sx, e[1][1] * sy)])}
        />
      ))}

      {/* Routes. */}
      {LINKS.map((l, i) => (
        <Path
          key={`link-${i}`}
          style={{ position: "absolute", left: 0, top: 0, opacity: pop(i + 1) }}
          fill="none"
          stroke="#38bdf8"
          strokeWidth="4"
          strokeLinecap="round"
          d={line([point(l[0][0] * sx, l[0][1] * sy), point(l[1][0] * sx, l[1][1] * sy)])}
        />
      ))}

      {SITES.map((s, i) => (
        <View
          key={`site-${i}`}
          style={{
            position: "absolute",
            left: (SITE_XY[i][0] - DOT) * sx,
            top: (SITE_XY[i][1] - DOT) * sy,
            width: DOT * 2 * sx,
            height: DOT * 2 * sy,
            backgroundColor: "#f472b6",
            opacity: pop(i),
          }}
        />
      ))}
      {SITES.map((s, i) => (
        <Text
          key={`name-${i}`}
          style={{
            position: "absolute",
            left: (SITE_XY[i][0] + DOT * 1.6) * sx,
            top: (SITE_XY[i][1] - NAME_FONT * 0.6) * sy,
            fontSize: NAME_FONT * sy,
            color: "#e2e8f0",
            opacity: pop(i),
          }}
        >
          {s.label}
        </Text>
      ))}
    </Scene>
  );
}
