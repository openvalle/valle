export const component = "stacked-bars";

const LABELS = ["MON", "TUE", "WED", "THU", "FRI"];
const SERIES = [
  { id: "core", label: "Core", color: "#38bdf8", values: [28, 34, 31, 42, 47] },
  { id: "plus", label: "Plus", color: "#8b5cf6", values: [18, 22, 26, 24, 31] },
  { id: "new", label: "New", color: "#2dd4bf", values: [8, 12, 16, 21, 18] },
];
const STACK = stack(SERIES.map((series) => series.values));
const DOMAIN = niceDomain(0, extent(STACK.positiveTotals)[1], 5);
const X = scaleBand({ range: [160, 930], count: LABELS.length, paddingInner: 0.32 });
const Y = scaleLinear({ domain: DOMAIN, range: [600, 150] });

function Segment(ctx, { seriesIndex, categoryIndex, color }) {
  const pair = STACK.layers[seriesIndex][categoryIndex];
  const top = Y.map(pair[1]);
  const bottom = Y.map(pair[0]);
  const reveal = interpolate(ctx.enter.progress - categoryIndex * 0.055 - seriesIndex * 0.035, [0, 0.52], [0, 1], { easing: "easeOut" });
  return <View key="segment" style={{ position: "absolute", left: X.band(categoryIndex)[0], top: bottom - (bottom - top) * reveal, width: X.bandwidth(), height: (bottom - top) * reveal, backgroundColor: color, borderRadius: seriesIndex === SERIES.length - 1 ? 12 : 2 }} />;
}

function LegendItem(ctx, { index, label, color }) {
  return (
    <View key="item" className="flex items-center" style={{ marginTop: index === 0 ? 0 : 20, width: 190, height: 34 }}>
      <View key="swatch" style={{ width: 16, height: 16, borderRadius: 5, backgroundColor: color }} />
      <Text key="label" style={{ marginLeft: 12, fontSize: 20, color: "#cbd5e1" }}>{label}</Text>
    </View>
  );
}

function StackedBarChart(ctx) {
  return (
    <View key="chart" className="absolute" style={{ left: 0, top: 0, width: 1280, height: 720 }}>
      {ticks(DOMAIN[0], DOMAIN[1], 5).map((value, index) => <View key={`grid-${index}`} style={{ position: "absolute", left: 160, top: Y.map(value), width: 770, height: 1, backgroundColor: "#334155" }} />)}
      {SERIES.map((series, seriesIndex) => LABELS.map((label, categoryIndex) => <Segment key={`${series.id}-${categoryIndex}`} seriesIndex={seriesIndex} categoryIndex={categoryIndex} color={series.color} />))}
      {LABELS.map((label, index) => <Text key={`category-${index}`} className="absolute" style={{ left: X.band(index)[0], top: 620, width: X.bandwidth(), fontSize: 18, color: "#94a3b8", textAlign: "center" }}>{label}</Text>)}
      <View key="legend" className="absolute" style={{ left: 1010, top: 190, width: 200, height: 230 }}>
        <Text key="heading" style={{ fontSize: 16, letterSpacing: 2, color: "#64748b" }}>SEGMENTS</Text>
        {SERIES.map((series, index) => <LegendItem key={series.id} index={index} label={series.label} color={series.color} />)}
      </View>
    </View>
  );
}

export default function StackedBarsFilm(ctx) {
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#0b1020" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 72, top: 44, fontSize: 18, color: "#a78bfa", letterSpacing: 2 }}>PREPARED STACK / DIVERGING READY</Text>
      <Text key="title" className="absolute" style={{ left: 72, top: 76, fontSize: 42, color: "#f8fafc" }}>Composition stays ordinary</Text>
      <StackedBarChart key="main" />
    </Scene>
  );
}
