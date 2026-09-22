export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

export const controls = {
  props: {
    accent: color({ default: "#22d3ee" }),
    area: color({ default: "#164e63" }),
    label: color({ default: "#cbd5e1" }),
  },
};

const DATA = [18, 27, 24, 41, 38, 56, 64, 58, 76];
const LABELS = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP"];
const X = scalePoint({ range: [225, 1695], count: DATA.length });
const Y = scaleLinear({ domain: niceDomain(0, extent(DATA)[1], 4), range: [885, 225] });
const POINTS = DATA.map((value, index) => point(X.at(index), Y.map(value)));
const TREND = line(POINTS);
const FILL = area(POINTS, 885);

function Grid(ctx, { color }) {
  return ticks(0, 80, 4).map((value, index) => (
    <View key={`line-${index}`} style={{ position: "absolute", left: 225, top: Y.map(value), width: 1470, height: 1.5, backgroundColor: color, opacity: 0.44 }} />
  ));
}

function Marker(ctx, { index, value, accent, label }) {
  const reveal = interpolate(ctx.progress - index * 0.045, [0, 0.45], [0, 1], { easing: "easeOut" });
  return (
    <View key="marker" style={{ position: "absolute", left: X.at(index) - 12, top: Y.map(value) - 12, width: 24, height: 24, borderRadius: 12, backgroundColor: accent, opacity: reveal, filter: `drop-shadow(0px 0px ${12 + reveal * 15}px ${accent})` }}>
      <Text key="label" style={{ position: "absolute", left: -42, top: 42, width: 108, fontSize: 24, color: label, textAlign: "center" }}>{LABELS[index]}</Text>
    </View>
  );
}

function Area(ctx, { pathValue, fill, opacity }) {
  return <Path key="area" d={pathValue} fill={fill} style={{ opacity }} />;
}

function Line(ctx, { pathValue, stroke, reveal }) {
  return <Path key="line" d={pathValue} fill="none" stroke={stroke} strokeWidth="9" strokeLinecap="round" trimEnd={reveal} />;
}

function LineChart(ctx, { accent, areaColor, label }) {
  const reveal = interpolate(ctx.progress, [0.08, 0.88], [0, 1], { easing: "easeInOut" });
  return (
    <View key="chart" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080 }}>
      <Grid key="grid" color="#334155" />
      <Area key="area-part" pathValue={FILL} fill={areaColor} opacity={reveal * 0.52} />
      <Line key="line-part" pathValue={TREND} stroke={accent} reveal={reveal} />
      {DATA.map((value, index) => <Marker key={`marker-${index}`} index={index} value={value} accent={accent} label={label} />)}
    </View>
  );
}

export default function LineChartFilm(ctx, props) {
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#071018" }}>
      <View key="glow" className="absolute" style={{ right: -180, top: -270, width: 900, height: 900, borderRadius: 450, backgroundColor: props.area, opacity: 0.28, filter: "blur(150px)" }} />
      <Text key="eyebrow" className="absolute" style={{ left: 108, top: 66, fontSize: 27, letterSpacing: 3, color: props.accent }}>SIGNAL / 09 MONTHS</Text>
      <Text key="title" className="absolute" style={{ left: 108, top: 114, fontSize: 63, color: props.label }}>Momentum without a second runtime</Text>
      <LineChart key="main" accent={props.accent} areaColor={props.area} label={props.label} />
    </Scene>
  );
}
