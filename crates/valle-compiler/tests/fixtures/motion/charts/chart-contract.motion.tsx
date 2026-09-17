export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "chart-contract";

export const controls = defineControls({
  props: {
    accent: color({ default: "#38bdf8" }),
    grid: color({ default: "#334155" }),
    label: color({ default: "#cbd5e1" }),
  },
});

const FROM = [28, 46, 38, 62];
const TO = [42, 31, 72, 88];
const LABELS = ["Q1", "Q2", "Q3", "Q4"];
const X = scaleBand({ range: [255, 1680], count: LABELS.length, paddingInner: 0.26 });
const Y_FROM = scaleLinear({ domain: [0, 80], range: [885, 195] });
const Y_TO = scaleLinear({ domain: [0, 100], range: [885, 195] });
const TICKS_FROM = ticks(0, 80, 4);
const TICKS_TO = ticks(0, 100, 4);
const TICK_Y_FROM = TICKS_FROM.map((value) => Y_FROM.map(value));
const TICK_Y_TO = TICKS_TO.map((value) => Y_TO.map(value));
const TOP_FROM = FROM.map((value) => Y_FROM.map(value));
const TOP_TO = TO.map((value) => Y_TO.map(value));
const BASE = 885;

function GridLine(ctx, { y, opacity, color }) {
  return <View key="line" style={{ position: "absolute", left: 255, top: y, width: 1425, height: 1.5, backgroundColor: color, opacity }} />;
}

function AxisLabel(ctx, { y, value, opacity, color }) {
  return <Text key="label" style={{ position: "absolute", left: 108, top: y - 18, width: 117, fontSize: 30, color, textAlign: "right", opacity }}>{`${value}`}</Text>;
}

function Axis(ctx, { values, positions, opacity, grid, label }) {
  return (
    <View key="axis" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080 }}>
      {values.map((value, index) => <GridLine key={`grid-${index}`} y={positions[index]} opacity={opacity} color={grid} />)}
      {values.map((value, index) => <AxisLabel key={`tick-${index}`} y={positions[index]} value={value} opacity={opacity} color={label} />)}
    </View>
  );
}

function Bar(ctx, { index, label, accent, labelColor, progress }) {
  const top = interpolate(progress, [0, 1], [TOP_FROM[index], TOP_TO[index]]);
  return (
    <View key="bar" style={{ position: "absolute", left: X.band(index)[0], top, width: X.bandwidth(), height: BASE - top, backgroundColor: accent }}>
      <Text key="label" style={{ marginTop: BASE - top + 27, width: X.bandwidth(), fontSize: 33, color: labelColor, textAlign: "center" }}>{label}</Text>
    </View>
  );
}

function Legend(ctx, { accent, labelColor }) {
  return (
    <View key="legend" className="absolute flex items-center" style={{ right: 108, top: 81, width: 330, height: 63 }}>
      <View key="swatch" style={{ width: 27, height: 27, borderRadius: 7.5, backgroundColor: accent }} />
      <Text key="copy" style={{ marginLeft: 18, fontSize: 30, color: labelColor }}>Quarterly signal</Text>
    </View>
  );
}

function BarChart(ctx, { accent, grid, label }) {
  const progress = ctx.hold.progress;
  return (
    <View key="chart" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080 }}>
      <Axis key="from-axis" values={TICKS_FROM} positions={TICK_Y_FROM} opacity={1 - progress} grid={grid} label={label} />
      <Axis key="to-axis" values={TICKS_TO} positions={TICK_Y_TO} opacity={progress} grid={grid} label={label} />
      {LABELS.map((name, index) => <Bar key={`bar-${index}`} index={index} label={name} accent={accent} labelColor={label} progress={progress} />)}
      <Legend key="legend-part" accent={accent} labelColor={label} />
    </View>
  );
}

export default function ChartContract(ctx, props) {
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#07111f" }}>
      <Text key="title" className="absolute" style={{ left: 108, top: 72, fontSize: 51, color: props.label }}>Prepared domains, fixed frame topology</Text>
      <BarChart key="main" accent={props.accent} grid={props.grid} label={props.label} />
    </Scene>
  );
}
