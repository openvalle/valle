export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

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
const LEFT = 132;
const TOP = 252;
const CELL_W = 129;
const CELL_H = 108;
const GAP = 12;
const LOAD = 0.74;
const GAUGE_START = deg(-130);
const GAUGE_SPAN = deg(260);
const GX = 1542;
const GY = 585;

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
        borderRadius: 15,
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
      <Text key="eyebrow" className="absolute" style={{ left: 108, top: 54, fontSize: 27, letterSpacing: 4.5, color: "#22d3ee" }}>
        LOAD / WEEK
      </Text>
      <Text key="title" className="absolute" style={{ left: 108, top: 102, fontSize: 57, color: "#e2e8f0" }}>
        Color is mapped once, the meter sweeps
      </Text>
      {HOURS.map((label, row) => (
        <Text
          key={`hour-${label}`}
          className="absolute"
          style={{ left: 42, top: TOP + row * (CELL_H + GAP) + 33, width: 72, fontSize: 24, color: "#64748b", textAlign: "right" }}
        >
          {label}
        </Text>
      ))}
      {DAYS.map((label, col) => (
        <Text
          key={`day-${label}`}
          className="absolute"
          style={{ left: LEFT + col * (CELL_W + GAP), top: TOP + 5 * (CELL_H + GAP) + 18, width: CELL_W, fontSize: 22.5, color: "#94a3b8", textAlign: "center" }}
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
          inner: 117,
          outer: 162,
          start: GAUGE_START,
          end: GAUGE_START + GAUGE_SPAN,
          cornerRadius: 12,
        })}
        fill="#1e293b"
        style={{ opacity: track }}
      />
      <Path
        key="gauge-value"
        d={sector({
          center: point(GX, GY),
          inner: 117,
          outer: 162,
          start: GAUGE_START,
          end: needle,
          cornerRadius: 12,
        })}
        fill="#22d3ee"
      />
      <Text
        key="gauge-label"
        className="absolute"
        style={{ left: GX - 105, top: GY - 27, width: 315, fontSize: 24, letterSpacing: 3, color: "#64748b", textAlign: "center" }}
      >
        CAPACITY
      </Text>
      <Text
        key="gauge-read"
        className="absolute"
        style={{ left: GX - 105, top: GY + 9, width: 315, fontSize: 48, color: "#ecfeff", textAlign: "center" }}
      >
        {formatPercent(LOAD, { decimals: 0 })}
      </Text>
    </Scene>
  );
}
