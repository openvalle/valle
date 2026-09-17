export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "dashboard-flip";

export const controls = defineControls({
  timing: { enterDuration: 0, exitDuration: 0 },
});

const layouts = defineLayoutStates({
  overview: {
    revenue: rect(105, 264, 817.5, 330),
    velocity: rect(952.5, 264, 862.5, 330),
    regions: rect(105, 624, 540, 345),
    quality: rect(675, 624, 1140, 345),
  },
  focus: {
    revenue: rect(105, 264, 1140, 705),
    velocity: rect(1275, 264, 540, 217.5),
    regions: rect(1275, 511.5, 540, 217.5),
    quality: rect(1275, 759, 540, 210),
  },
});

const BARS = [87, 123, 100.5, 141, 114, 162, 189, 177, 217.5];

export default function DashboardFlip(ctx) {
  const p = interpolate(ctx.hold.progress, [0, 0.24, 0.76, 1], [0, 1, 1, 0], { easing: "easeInOut" });
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#080b12" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 105, top: 72, fontSize: 22.5, letterSpacing: 6, color: "#f59e0b" }}>CONTROL ROOM / FLIP LAYOUT</Text>
      <Text key="heading" className="absolute" style={{ left: 102, top: 123, fontSize: 75, color: "#f8fafc" }}>From overview to decisive focus.</Text>
      <Text key="state" className="absolute" style={{ right: 105, top: 93, fontSize: 27, color: "#64748b" }}>LAYOUT PROGRESS {formatPercent(p, { decimals: 0 })}</Text>

      <View key="revenue" layoutId="revenue" style={{ borderStyle: "solid", layoutTransition: flip(layouts, "overview", "focus", p), borderRadius: 36, backgroundColor: "#111827", borderWidth: 1.5, borderColor: "#334155", overflow: "hidden" }}>
        <Text key="revenue-label" className="absolute" style={{ left: 39, top: 33, fontSize: 22.5, letterSpacing: 3, color: "#94a3b8" }}>REVENUE VELOCITY</Text>
        <Text key="revenue-value" className="absolute" style={{ left: 36, top: 78, fontSize: 87, color: "#f8fafc" }}>$4.82M</Text>
        <Text key="revenue-delta" className="absolute" style={{ right: 37.5, top: 91.5, fontSize: 28.5, color: "#34d399" }}>+18.4%</Text>
        {BARS.map((height, i) => (
          <View key={`bar-${i}`} className="absolute" style={{ left: 42 + i * (83 + p * 42), bottom: 36, width: 63, height: height * (0.55 + p * 0.65), borderRadius: 12, backgroundColor: i === 8 ? "#f59e0b" : "#334155" }} />
        ))}
      </View>

      <View key="velocity" layoutId="velocity" style={{ borderStyle: "solid", layoutTransition: flip(layouts, "overview", "focus", p), borderRadius: 36, backgroundColor: "#0f172a", borderWidth: 1.5, borderColor: "#273449" }}>
        <Text key="v-label" className="absolute" style={{ left: 36, top: 33, fontSize: 22.5, color: "#94a3b8" }}>DEPLOY VELOCITY</Text>
        <Text key="v-value" className="absolute" style={{ left: 33, top: 78, fontSize: 72, color: "#67e8f9" }}>42.6×</Text>
      </View>
      <View key="regions" layoutId="regions" style={{ borderStyle: "solid", layoutTransition: flip(layouts, "overview", "focus", p), borderRadius: 36, backgroundColor: "#101827", borderWidth: 1.5, borderColor: "#273449" }}>
        <Text key="r-label" className="absolute" style={{ left: 36, top: 33, fontSize: 22.5, color: "#94a3b8" }}>ACTIVE REGIONS</Text>
        <Text key="r-value" className="absolute" style={{ left: 33, top: 82.5, fontSize: 78, color: "#a78bfa" }}>18 / 20</Text>
      </View>
      <View key="quality" layoutId="quality" style={{ borderStyle: "solid", layoutTransition: flip(layouts, "overview", "focus", p), borderRadius: 36, backgroundColor: "#0f172a", borderWidth: 1.5, borderColor: "#273449" }}>
        <Text key="q-label" className="absolute" style={{ left: 36, top: 33, fontSize: 22.5, color: "#94a3b8" }}>FRAME CONFIDENCE</Text>
        <Text key="q-value" className="absolute" style={{ left: 33, top: 82.5, fontSize: 78, color: "#34d399" }}>{formatPercent(0.999, { decimals: 1 })}</Text>
      </View>
    </Scene>
  );
}
