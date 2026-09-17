export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "stacked-bars";

const LABELS = ["MON", "TUE", "WED", "THU", "FRI"];
const SERIES = [
  { id: "core", label: "Core", color: "#38bdf8", values: [28, 34, 31, 42, 47] },
  { id: "plus", label: "Plus", color: "#8b5cf6", values: [18, 22, 26, 24, 31] },
  { id: "new", label: "New", color: "#2dd4bf", values: [8, 12, 16, 21, 18] },
];
const STACK = stack(SERIES.map((series) => series.values));
const DOMAIN = niceDomain(0, extent(STACK.positiveTotals)[1], 5);
const X = scaleBand({ range: [240, 1395], count: LABELS.length, paddingInner: 0.32 });
const Y = scaleLinear({ domain: DOMAIN, range: [900, 225] });

function Segment(ctx, { seriesIndex, categoryIndex, color }) {
  const pair = STACK.layers[seriesIndex][categoryIndex];
  const top = Y.map(pair[1]);
  const bottom = Y.map(pair[0]);
  const reveal = interpolate(ctx.enter.progress - categoryIndex * 0.055 - seriesIndex * 0.035, [0, 0.52], [0, 1], { easing: "easeOut" });
  return <View key="segment" style={{ position: "absolute", left: X.band(categoryIndex)[0], top: bottom - (bottom - top) * reveal, width: X.bandwidth(), height: (bottom - top) * reveal, backgroundColor: color, borderRadius: seriesIndex === SERIES.length - 1 ? 18 : 3 }} />;
}

function LegendItem(ctx, { index, label, color }) {
  return (
    <View key="item" className="flex items-center" style={{ marginTop: index === 0 ? 0 : 20, width: 285, height: 51 }}>
      <View key="swatch" style={{ width: 24, height: 24, borderRadius: 7.5, backgroundColor: color }} />
      <Text key="label" style={{ marginLeft: 18, fontSize: 30, color: "#cbd5e1" }}>{label}</Text>
    </View>
  );
}

function StackedBarChart(ctx) {
  return (
    <View key="chart" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080 }}>
      {ticks(DOMAIN[0], DOMAIN[1], 5).map((value, index) => <View key={`grid-${index}`} style={{ position: "absolute", left: 240, top: Y.map(value), width: 1155, height: 1.5, backgroundColor: "#334155" }} />)}
      {SERIES.map((series, seriesIndex) => LABELS.map((label, categoryIndex) => <Segment key={`${series.id}-${categoryIndex}`} seriesIndex={seriesIndex} categoryIndex={categoryIndex} color={series.color} />))}
      {LABELS.map((label, index) => <Text key={`category-${index}`} className="absolute" style={{ left: X.band(index)[0], top: 930, width: X.bandwidth(), fontSize: 27, color: "#94a3b8", textAlign: "center" }}>{label}</Text>)}
      <View key="legend" className="absolute" style={{ left: 1515, top: 285, width: 300, height: 345 }}>
        <Text key="heading" style={{ fontSize: 24, letterSpacing: 3, color: "#64748b" }}>SEGMENTS</Text>
        {SERIES.map((series, index) => <LegendItem key={series.id} index={index} label={series.label} color={series.color} />)}
      </View>
    </View>
  );
}

export default function StackedBarsFilm(ctx) {
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#0b1020" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 108, top: 66, fontSize: 27, color: "#a78bfa", letterSpacing: 3 }}>PREPARED STACK / DIVERGING READY</Text>
      <Text key="title" className="absolute" style={{ left: 108, top: 114, fontSize: 63, color: "#f8fafc" }}>Composition stays ordinary</Text>
      <StackedBarChart key="main" />
    </Scene>
  );
}
