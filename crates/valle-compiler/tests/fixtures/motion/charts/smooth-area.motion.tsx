export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "smooth-area";

export const controls = defineControls({
  props: {
    accent: color({ default: "#22d3ee" }),
    fill: color({ default: "#155e75" }),
    ink: color({ default: "#e2e8f0" }),
  },
});

const DATA = [16, 22, 19, 34, 31, 48, 44, 61, 58, 72];
const LABELS = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT"];
const DOMAIN = niceDomain(0, extent(DATA)[1], 4);
const X = scalePoint({ range: [225, 1770], count: DATA.length });
const Y = scaleLinear({ domain: DOMAIN, range: [885, 225] });
const BASELINE = Y.map(DOMAIN[0]);
const TREND = curve(
  DATA.map((value, index) => point(X.at(index), Y.map(value))),
  { type: "monotoneX" },
);
const FILL = area(TREND, BASELINE);
const TICKS = ticks(DOMAIN[0], DOMAIN[1], 4);

function Marker(ctx, { index, value, accent, ink }) {
  const show = interpolate(ctx.progress - 0.42 - index * 0.035, [0, 0.28], [0, 1], {
    easing: "easeOut",
  });
  return (
    <View
      key="dot"
      className="absolute"
      style={{
        left: X.at(index) - 10.5,
        top: Y.map(value) - 10.5,
        width: 21,
        height: 21,
        borderRadius: 10.5,
        backgroundColor: accent,
        opacity: show,
      }}
    >
      <Text
        key="month"
        className="absolute"
        style={{ left: -42, top: 33, width: 105, fontSize: 22.5, color: ink, textAlign: "center", opacity: 0.8 }}
      >
        {LABELS[index]}
      </Text>
    </View>
  );
}

export default function SmoothArea(ctx, props) {
  const draw = interpolate(ctx.progress, [0.08, 0.78], [0, 1], { easing: "easeInOut" });
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#071018" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 108, top: 60, fontSize: 27, letterSpacing: 4.5, color: props.accent }}>
        TREND / MONOTONE
      </Text>
      <Text key="title" className="absolute" style={{ left: 108, top: 108, fontSize: 60, color: props.ink }}>
        The line is fitted once, then drawn
      </Text>
      {TICKS.map((value, index) => (
        <View
          key={`grid-${index}`}
          className="absolute"
          style={{ left: 225, top: Y.map(value), width: 1545, height: 1.5, backgroundColor: "#1e293b" }}
        />
      ))}
      {TICKS.map((value, index) => (
        <Text
          key={`tick-${index}`}
          className="absolute"
          style={{ left: 72, top: Y.map(value) - 18, width: 135, fontSize: 27, color: "#64748b", textAlign: "right" }}
        >
          {`${formatNumber(value, { decimals: 0 })}`}
        </Text>
      ))}
      <Path key="fill" d={FILL} fill={props.fill} style={{ opacity: draw * 0.55 }} />
      <Path
        key="stroke"
        d={TREND}
        fill="none"
        stroke={props.accent}
        strokeWidth="9"
        strokeLinecap="round"
        trimEnd={draw}
      />
      {DATA.map((value, index) => (
        <Marker key={`m-${index}`} index={index} value={value} accent={props.accent} ink={props.ink} />
      ))}
    </Scene>
  );
}
