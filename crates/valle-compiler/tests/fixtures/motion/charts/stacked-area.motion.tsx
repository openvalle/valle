export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "stacked-area";

const LABELS = ["W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];
const SERIES = [
  { id: "base", label: "Base", color: "#155e75", values: [18, 22, 20, 26, 24, 30, 28, 34] },
  { id: "plus", label: "Plus", color: "#0e7490", values: [10, 12, 16, 14, 18, 16, 22, 20] },
];
const STACK = stack(SERIES.map((series) => series.values));
const DOMAIN = niceDomain(0, extent(STACK.positiveTotals)[1], 4);
const X = scalePoint({ range: [225, 1470], count: LABELS.length });
const Y = scaleLinear({ domain: DOMAIN, range: [885, 240] });
const ZERO = curve(
  LABELS.map((_, index) => point(X.at(index), Y.map(0))),
  { type: "monotoneX" },
);
const MID = curve(
  STACK.layers[0].map((pair, index) => point(X.at(index), Y.map(pair[1]))),
  { type: "monotoneX" },
);
const TOP = curve(
  STACK.layers[1].map((pair, index) => point(X.at(index), Y.map(pair[1]))),
  { type: "monotoneX" },
);
const BASE_BAND = areaBand(MID, ZERO);
const PLUS_BAND = areaBand(TOP, MID);

function LegendItem(ctx, { index, label, color }) {
  return (
    <View key="item" className="flex items-center" style={{ marginTop: index === 0 ? 0 : 18, width: 270, height: 48 }}>
      <View key="swatch" style={{ width: 24, height: 24, borderRadius: 6, backgroundColor: color }} />
      <Text key="name" style={{ marginLeft: 18, fontSize: 30, color: "#cbd5e1" }}>{label}</Text>
    </View>
  );
}

export default function StackedArea(ctx) {
  const reveal = interpolate(ctx.progress, [0.06, 0.82], [0, 1], { easing: "easeInOut" });
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#07111a" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 108, top: 60, fontSize: 27, letterSpacing: 4.5, color: "#22d3ee" }}>
        STACK / SHARED EDGE
      </Text>
      <Text key="title" className="absolute" style={{ left: 108, top: 108, fontSize: 60, color: "#e2e8f0" }}>
        Layers meet on one prepared curve
      </Text>
      {ticks(DOMAIN[0], DOMAIN[1], 4).map((value, index) => (
        <View
          key={`grid-${index}`}
          className="absolute"
          style={{ left: 225, top: Y.map(value), width: 1245, height: 1.5, backgroundColor: "#1e293b" }}
        />
      ))}
      <Path key="base" d={BASE_BAND} fill={SERIES[0].color} style={{ opacity: 0.42 + reveal * 0.4 }} />
      <Path key="plus" d={PLUS_BAND} fill={SERIES[1].color} style={{ opacity: 0.35 + reveal * 0.45 }} />
      <Path key="mid-line" d={MID} fill="none" stroke="#67e8f9" strokeWidth="4.5" trimEnd={reveal} />
      <Path key="top-line" d={TOP} fill="none" stroke="#a5f3fc" strokeWidth="4.5" trimEnd={reveal} />
      {LABELS.map((label, index) => (
        <Text
          key={`lab-${index}`}
          className="absolute"
          style={{ left: X.at(index) - 36, top: 918, width: 108, fontSize: 24, color: "#94a3b8", textAlign: "center" }}
        >
          {label}
        </Text>
      ))}
      <View key="legend" className="absolute" style={{ left: 1560, top: 300, width: 270, height: 180 }}>
        {SERIES.map((series, index) => (
          <LegendItem key={series.id} index={index} label={series.label} color={series.color} />
        ))}
      </View>
    </Scene>
  );
}
