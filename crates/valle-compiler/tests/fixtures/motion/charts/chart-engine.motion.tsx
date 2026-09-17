export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "chart-engine";

const VALUES = [12, 28, 18, 32];
const LABELS = ["A", "B", "C", "D"];
const SERIES = [
  [8, 12, 10, 14],
  [4, 9, 6, 11],
];
const STACK = stack(SERIES);
const LINE = [18, 24, 21, 36, 32, 44];
const PIE = pie(VALUES, { startAngle: deg(-90), endAngle: deg(-90) + TAU });
const SEQ = scaleSequential({ domain: [0, 44], colors: ["#0f172a", "#38bdf8"] });
const GROUP = scaleBand({ range: [120, 540], count: LABELS.length, paddingInner: 0.28 });
const INNER = scaleBand({ range: [0, GROUP.bandwidth()], count: 2, paddingInner: 0.16 });
const Y = scaleLinear({ domain: [0, 40], range: [510, 120] });
const X_LINE = scalePoint({ range: [630, 1140], count: LINE.length });
const TREND = curve(
  LINE.map((value, index) => point(X_LINE.at(index), Y.map(value))),
  { type: "monotoneX" },
);
const FILL = area(TREND, 510);
const UPPER = curve(
  STACK.layers[1].map((pair, index) => point(X_LINE.at(index), Y.map(pair[1]))),
  { type: "monotoneX" },
);
const LOWER = curve(
  STACK.layers[1].map((pair, index) => point(X_LINE.at(index), Y.map(pair[0]))),
  { type: "monotoneX" },
);
const BAND = areaBand(UPPER, LOWER);

export default function ChartEngine(ctx) {
  const needle = interpolate(ctx.progress, [0, 1], [deg(-120), deg(120)]);
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#020617" }}>
      <View key="plot-surface" className="absolute" style={{ borderStyle: "solid", left: 60, top: 63, width: 1140, height: 930, borderRadius: 39, backgroundColor: "#0c1830", borderWidth: 1.5, borderColor: "#263959" }} />
      <View key="summary" className="absolute" style={{ borderStyle: "solid", left: 1248, top: 63, width: 609, height: 930, borderRadius: 39, backgroundColor: "#101b33", borderWidth: 1.5, borderColor: "#334665" }}>
        <Text key="eyebrow" className="absolute" style={{ left: 54, top: 60, fontSize: 24, letterSpacing: 4.5, color: "#67e8f9" }}>CHART ENGINE / 05</Text>
        <Text key="heading" className="absolute" style={{ left: 51, top: 129, width: 495, fontSize: 51, color: "#f8fafc" }}>One data set, many marks.</Text>
        <View key="rule" className="absolute" style={{ left: 54, top: 306, width: 501, height: 1.5, backgroundColor: "#334665" }} />
        <Text key="bars-label" className="absolute" style={{ left: 54, top: 363, fontSize: 31.5, color: "#a5b4fc" }}>01  Grouped bars</Text>
        <Text key="area-label" className="absolute" style={{ left: 54, top: 453, fontSize: 31.5, color: "#67e8f9" }}>02  Smooth area</Text>
        <Text key="pie-label" className="absolute" style={{ left: 54, top: 543, fontSize: 31.5, color: "#a5b4fc" }}>03  Donut share</Text>
        <Text key="gauge-label" className="absolute" style={{ left: 54, top: 633, fontSize: 31.5, color: "#67e8f9" }}>04  Gauge sweep</Text>
        <Text key="footer" className="absolute" style={{ left: 54, bottom: 63, fontSize: 25.5, color: "#94a3b8" }}>All geometry is prepared once.</Text>
      </View>
      {LABELS.map((label, category) =>
        SERIES.map((series, seriesIndex) => (
          <View
            key={`${label}-${seriesIndex}`}
            style={{
              position: "absolute",
              left: GROUP.band(category)[0] + INNER.band(seriesIndex)[0],
              top: Y.map(series[category]),
              width: INNER.bandwidth(),
              height: Y.map(0) - Y.map(series[category]),
              backgroundColor: seriesIndex === 0 ? "#38bdf8" : "#a78bfa",
            }}
          />
        )),
      )}
      <Path key="area" d={FILL} fill="#164e63" />
      <Path key="line" d={TREND} fill="none" stroke="#22d3ee" strokeWidth="6" trimEnd={ctx.progress} />
      <Path key="band" d={BAND} fill="#334155" />
      {PIE.map((slice, index) => (
        <Path
          key={`pie-${index}`}
          d={sector({
            center: point(240, 780),
            inner: 36,
            outer: 105,
            start: slice.startAngle,
            end: interpolate(ctx.progress, [0, 1], [slice.startAngle, slice.endAngle], {
              easing: "easeOut",
            }),
          })}
          fill={SEQ.map(VALUES[index])}
        />
      ))}
      <Path
        key="gauge"
        d={sector({
          center: point(960, 780),
          inner: 54,
          outer: 78,
          start: deg(-120),
          end: needle,
        })}
        fill="#22d3ee"
      />
      <View
        key="radar"
        style={{
          position: "absolute",
          left: 960 + 60 * Math.cos(deg(-90)),
          top: 300 + 60 * Math.sin(deg(-90)),
          width: 12,
          height: 12,
          backgroundColor: SEQ.map(LINE[LINE.length - 1]),
        }}
      />
    </Scene>
  );
}
