export const component = "dashboard-flip";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const layouts = defineLayoutStates({
  overview: {
    revenue: rect(70, 176, 545, 220),
    velocity: rect(635, 176, 575, 220),
    regions: rect(70, 416, 360, 230),
    quality: rect(450, 416, 760, 230),
  },
  focus: {
    revenue: rect(70, 176, 760, 470),
    velocity: rect(850, 176, 360, 145),
    regions: rect(850, 341, 360, 145),
    quality: rect(850, 506, 360, 140),
  },
});

const BARS = [58, 82, 67, 94, 76, 108, 126, 118, 145];

export default function DashboardFlip(ctx) {
  const p = interpolate(ctx.hold.progress, [0, 0.24, 0.76, 1], [0, 1, 1, 0], { easing: "easeInOut" });
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#080b12" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 70, top: 48, fontSize: 15, letterSpacing: 4, color: "#f59e0b" }}>CONTROL ROOM / FLIP LAYOUT</Text>
      <Text key="heading" className="absolute" style={{ left: 68, top: 82, fontSize: 50, color: "#f8fafc" }}>From overview to decisive focus.</Text>
      <Text key="state" className="absolute" style={{ right: 70, top: 62, fontSize: 18, color: "#64748b" }}>LAYOUT PROGRESS {formatPercent(p, { decimals: 0 })}</Text>

      <View key="revenue" layoutId="revenue" style={{ borderStyle: "solid", layoutTransition: flip(layouts, "overview", "focus", p), borderRadius: 24, backgroundColor: "#111827", borderWidth: 1, borderColor: "#334155", overflow: "hidden" }}>
        <Text key="revenue-label" className="absolute" style={{ left: 26, top: 22, fontSize: 15, letterSpacing: 2, color: "#94a3b8" }}>REVENUE VELOCITY</Text>
        <Text key="revenue-value" className="absolute" style={{ left: 24, top: 52, fontSize: 58, color: "#f8fafc" }}>$4.82M</Text>
        <Text key="revenue-delta" className="absolute" style={{ right: 25, top: 61, fontSize: 19, color: "#34d399" }}>+18.4%</Text>
        {BARS.map((height, i) => (
          <View key={`bar-${i}`} className="absolute" style={{ left: 28 + i * 67, bottom: 24, width: 42, height: height * (0.55 + p * 0.65), borderRadius: 8, backgroundColor: i === 8 ? "#f59e0b" : "#334155" }} />
        ))}
      </View>

      <View key="velocity" layoutId="velocity" style={{ borderStyle: "solid", layoutTransition: flip(layouts, "overview", "focus", p), borderRadius: 24, backgroundColor: "#0f172a", borderWidth: 1, borderColor: "#273449" }}>
        <Text key="v-label" className="absolute" style={{ left: 24, top: 22, fontSize: 15, color: "#94a3b8" }}>DEPLOY VELOCITY</Text>
        <Text key="v-value" className="absolute" style={{ left: 22, top: 52, fontSize: 48, color: "#67e8f9" }}>42.6×</Text>
      </View>
      <View key="regions" layoutId="regions" style={{ borderStyle: "solid", layoutTransition: flip(layouts, "overview", "focus", p), borderRadius: 24, backgroundColor: "#101827", borderWidth: 1, borderColor: "#273449" }}>
        <Text key="r-label" className="absolute" style={{ left: 24, top: 22, fontSize: 15, color: "#94a3b8" }}>ACTIVE REGIONS</Text>
        <Text key="r-value" className="absolute" style={{ left: 22, top: 55, fontSize: 52, color: "#a78bfa" }}>18 / 20</Text>
      </View>
      <View key="quality" layoutId="quality" style={{ borderStyle: "solid", layoutTransition: flip(layouts, "overview", "focus", p), borderRadius: 24, backgroundColor: "#0f172a", borderWidth: 1, borderColor: "#273449" }}>
        <Text key="q-label" className="absolute" style={{ left: 24, top: 22, fontSize: 15, color: "#94a3b8" }}>FRAME CONFIDENCE</Text>
        <Text key="q-value" className="absolute" style={{ left: 22, top: 55, fontSize: 52, color: "#34d399" }}>{formatPercent(0.999, { decimals: 1 })}</Text>
      </View>
    </Scene>
  );
}
