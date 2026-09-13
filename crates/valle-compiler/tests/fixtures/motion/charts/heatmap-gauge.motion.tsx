export const component = "heatmap-gauge";

const DAYS = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];
const HOURS = ["9", "12", "15", "18", "21"];
const CELLS = [
  [12, 18, 40, 62, 28, 10, 8],
  [16, 34, 70, 88, 44, 14, 9],
  [22, 48, 80, 96, 58, 18, 11],
  [14, 30, 54, 72, 36, 12, 7],
  [8, 14, 24, 32, 16, 6, 4],
];
const COLOR = scaleSequential({
  domain: [0, 100],
  colors: ["#0b1324", "#155e75", "#22d3ee", "#ecfeff"],
});
const LEFT = 88;
const TOP = 168;
const CELL_W = 86;
const CELL_H = 72;
const GAP = 8;
const LOAD = 0.74;
const GAUGE_START = deg(-130);
const GAUGE_SPAN = deg(260);
const GX = 1028;
const GY = 390;

function Cell(ctx, { row, col, value }) {
  const show = interpolate(ctx.progress - (row * 7 + col) * 0.012, [0, 0.28], [0, 1], {
    easing: "easeOut",
  });
  return (
    <View
      key="cell"
      className="absolute"
      style={{
        left: LEFT + col * (CELL_W + GAP),
        top: TOP + row * (CELL_H + GAP),
        width: CELL_W,
        height: CELL_H,
        borderRadius: 10,
        backgroundColor: COLOR.map(value),
        opacity: 0.22 + show * 0.78,
      }}
    />
  );
}

export default function HeatmapGauge(ctx) {
  const needle = interpolate(ctx.progress, [0.15, 0.85], [GAUGE_START, GAUGE_START + GAUGE_SPAN * LOAD], {
    easing: "easeOut",
  });
  const track = interpolate(ctx.progress, [0.08, 0.4], [0, 1], { easing: "easeOut" });
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#050b14" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 72, top: 36, fontSize: 18, letterSpacing: 3, color: "#22d3ee" }}>
        LOAD / WEEK
      </Text>
      <Text key="title" className="absolute" style={{ left: 72, top: 68, fontSize: 38, color: "#e2e8f0" }}>
        Color is mapped once, the meter sweeps
      </Text>
      {HOURS.map((label, row) => (
        <Text
          key={`hour-${label}`}
          className="absolute"
          style={{ left: 28, top: TOP + row * (CELL_H + GAP) + 22, width: 48, fontSize: 16, color: "#64748b", textAlign: "right" }}
        >
          {label}
        </Text>
      ))}
      {DAYS.map((label, col) => (
        <Text
          key={`day-${label}`}
          className="absolute"
          style={{ left: LEFT + col * (CELL_W + GAP), top: TOP + 5 * (CELL_H + GAP) + 12, width: CELL_W, fontSize: 15, color: "#94a3b8", textAlign: "center" }}
        >
          {label}
        </Text>
      ))}
      {CELLS.map((row, rowIndex) =>
        row.map((value, col) => (
          <Cell key={`${rowIndex}-${col}`} row={rowIndex} col={col} value={value} />
        )),
      )}
      <Path
        key="gauge-track"
        d={sector({
          center: point(GX, GY),
          inner: 78,
          outer: 108,
          start: GAUGE_START,
          end: GAUGE_START + GAUGE_SPAN,
          cornerRadius: 8,
        })}
        fill="#1e293b"
        style={{ opacity: track }}
      />
      <Path
        key="gauge-value"
        d={sector({
          center: point(GX, GY),
          inner: 78,
          outer: 108,
          start: GAUGE_START,
          end: needle,
          cornerRadius: 8,
        })}
        fill="#22d3ee"
      />
      <Text
        key="gauge-label"
        className="absolute"
        style={{ left: GX - 70, top: GY - 18, width: 140, fontSize: 16, letterSpacing: 2, color: "#64748b", textAlign: "center" }}
      >
        CAPACITY
      </Text>
      <Text
        key="gauge-read"
        className="absolute"
        style={{ left: GX - 70, top: GY + 6, width: 140, fontSize: 32, color: "#ecfeff", textAlign: "center" }}
      >
        {formatPercent(LOAD, { decimals: 0 })}
      </Text>
    </Scene>
  );
}
