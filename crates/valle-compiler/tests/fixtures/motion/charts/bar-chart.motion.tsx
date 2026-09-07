// Bar chart fixture.
//

//
// Derive geometry from public compute functions and scale it to the viewport.
//
// Canvas-to-viewport mapping.
//
// Compute functions run during preparation; viewport dimensions are frame-time inputs.
//
// Prepare scales and ticks in fixed canvas coordinates, then apply SX and SY at frame time.
export const component = "bar-chart";

// Input data.
//
// Keep labels and values in one array to prevent mismatched lengths.
const SERIES = [
  { label: "Q1", value: 128 },
  { label: "Q2", value: 342 },
  { label: "Q3", value: 267 },
  { label: "Q4", value: 489 },
  { label: "Q5", value: 411 },
  { label: "Q6", value: 523 },
];
const DATA = SERIES.map((d) => d.value);

// Fixed canvas coordinates, scaled to the viewport at frame time.
const CANVAS_W = 1920;
const CANVAS_H = 1080;

const PLOT_LEFT = CANVAS_W * 0.08;
const PLOT_RIGHT = CANVAS_W * 0.96;
const PLOT_TOP = CANVAS_H * 0.14;
const PLOT_BOTTOM = CANVAS_H * 0.86;
const TICK_COUNT = 5;

// Round the domain to useful tick boundaries.
const [LO, HI] = niceDomain(0, extent(DATA)[1], TICK_COUNT);
const TICKS = ticks(LO, HI, TICK_COUNT);

const X = scaleBand({
  range: [PLOT_LEFT, PLOT_RIGHT],
  count: DATA.length,
  paddingInner: 0.28,
});
const Y = scaleLinear({ domain: [LO, HI], range: [PLOT_BOTTOM, PLOT_TOP] });

// Scales are evaluated during preparation.
const BAR_W = X.bandwidth();
const BASELINE = Y.map(LO);
const BAR_X = DATA.map((v, i) => X.band(i)[0]);
const BAR_TOP = DATA.map((v) => Y.map(v));
const BAR_H = DATA.map((v) => BASELINE - Y.map(v));
const TICK_Y = TICKS.map((t) => Y.map(t));
const AXIS_FONT = CANVAS_H * 0.022;
const LABEL_FONT = CANVAS_H * 0.024;

export default function BarChart(ctx) {
  // The viewport supplies the frame-time scale factors.
  const sx = ctx.viewport.width / CANVAS_W;
  const sy = ctx.viewport.height / CANVAS_H;

  // Prepare staggered bar entrance timing.
  const grow = (i) => interpolate(ctx.enter.progress, [i * 0.08, i * 0.08 + 0.45], [0, 1]);

  return (
    <Scene className="h-full w-full bg-slate-950">
      {/* Grid lines follow the computed ticks. */}
      {TICKS.map((t, i) => (
        <View
          key={`grid-${i}`}
          style={{
            position: "absolute",
            left: PLOT_LEFT * sx,
            top: TICK_Y[i] * sy,
            width: (PLOT_RIGHT - PLOT_LEFT) * sx,
            height: 1,
            backgroundColor: "#1e293b",
          }}
        />
      ))}
      {TICKS.map((t, i) => (
        <Text
          key={`tick-${i}`}
          style={{
            position: "absolute",
            left: 0,
            top: (TICK_Y[i] - AXIS_FONT * 0.6) * sy,
            width: (PLOT_LEFT - CANVAS_W * 0.012) * sx,
            fontSize: AXIS_FONT * sy,
            color: "#64748b",
            textAlign: "right",
          }}
        >
          {`${t}`}
        </Text>
      ))}

      {/* Scale prepared bar geometry; animate opacity and height. */}
      {DATA.map((v, i) => (
        <View
          key={`bar-${i}`}
          style={{
            position: "absolute",
            left: BAR_X[i] * sx,
            top: BAR_TOP[i] * sy,
            width: BAR_W * sx,
            height: BAR_H[i] * sy,
            backgroundColor: "#38bdf8",
            opacity: grow(i),
          }}
        />
      ))}

      {/* Center labels within their bands. */}
      {SERIES.map((d, i) => (
        <Text
          key={`label-${i}`}
          style={{
            position: "absolute",
            left: BAR_X[i] * sx,
            top: (BASELINE + CANVAS_H * 0.02) * sy,
            width: BAR_W * sx,
            fontSize: LABEL_FONT * sy,
            color: "#94a3b8",
            textAlign: "center",
          }}
        >
          {d.label}
        </Text>
      ))}

      {/* Use a Path baseline to exercise geometry primitives. */}
      <Path
        key="axis"
        style={{ position: "absolute", left: 0, top: 0 }}
        fill="none"
        stroke="#334155"
        strokeWidth="3"
        d={line([
          point(PLOT_LEFT * sx, BASELINE * sy),
          point(PLOT_RIGHT * sx, BASELINE * sy),
        ])}
      />
    </Scene>
  );
}
