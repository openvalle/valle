export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "vector-turbulence-proxy";

export const controls = defineControls({
  timing: { enterDuration: 0, exitDuration: 0 },
});

const ribbons = defineRepeater({ count: 19, keyPrefix: "ribbon" });
const riders = defineRepeater({ count: 24, keyPrefix: "rider" });
const FLOW_A = path("M 135 558 C 345 198 621 885 864 513 C 1113 135 1353 858 1785 468");
const FLOW_B = path("M 135 480 C 381 861 606 129 903 585 C 1155 972 1422 177 1785 576");

export default function VectorTurbulenceProxy(ctx) {
  const t = ctx.hold.progress;
  const phase = (sin(t * 12.566370614 - 1.2) + 1) * 0.5;
  const flow = morphPath(FLOW_A, FLOW_B, phase);
  const reveal = interpolate(t, [0, 0.2], [0, 1], { easing: "easeOut" });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#f4f1e8" }}>
      <View key="ink-wash" className="absolute" style={{ left: 217.5, top: 247.5, width: 1485, height: 630, borderRadius: 315, backgroundColor: "#fb718522", filter: "blur(102px)" }} />
      <Text key="kicker" className="absolute" style={{ left: 87, top: 64.5, fontSize: 22.5, letterSpacing: 6, color: "#dc2626", opacity: reveal }}>VECTOR DEFORMATION</Text>
      <Text key="title" className="absolute" style={{ left: 81, top: 117, width: 1680, fontSize: 78, color: "#171717", opacity: reveal }}>Turbulence, without a raster effect.</Text>
      <Text key="note" className="absolute" style={{ left: 87, top: 214.5, width: 1350, fontSize: 27, color: "#57534e", opacity: reveal }}>A fixed-topology path morph drives nineteen deterministic offsets.</Text>

      {ribbons.map((ribbon) => (
        <Path key={ribbon.key} d={offsetPath(flow, (ribbon.index - 9) * 14)} fill="none" stroke={ribbon.index % 3 === 0 ? "#111827" : ribbon.index % 3 === 1 ? "#ef4444" : "#2563eb"} strokeWidth={ribbon.index === 9 ? 4 : 1.4} strokeLinecap="round" style={{ opacity: 0.12 + (1 - ribbon.progress) * 0.58 }} />
      ))}

      {riders.map((rider) => (
        <View key={rider.key} className="absolute" style={{ width: 7.5 + (1 - rider.progress) * 12, height: 7.5 + (1 - rider.progress) * 12, borderRadius: 12, backgroundColor: rider.index % 2 === 0 ? "#111827" : "#ef4444", opacity: 0.22 + (1 - rider.progress) * 0.72, filter: "drop-shadow(0px 0px 9px #ffffff)", motionPath: follow(flow, trail(t * 1.35, rider.index, { gap: 0.035, mode: "wrap" }), { rotate: "auto" }) }} />
      ))}

      <View key="legend" className="absolute flex items-center" style={{ borderStyle: "solid", left: 87, top: 939, width: 1746, height: 81, borderTopWidth: 1.5, borderColor: "#a8a29e" }}>
        <Text key="legend-a" style={{ fontSize: 21, letterSpacing: 3, color: "#171717" }}>MORPHPATH</Text>
        <Text key="legend-plus" style={{ marginLeft: 27, fontSize: 27, color: "#dc2626" }}>+</Text>
        <Text key="legend-b" style={{ marginLeft: 27, fontSize: 21, letterSpacing: 3, color: "#171717" }}>OFFSETPATH</Text>
        <Text key="legend-result" style={{ marginLeft: 63, fontSize: 21, letterSpacing: 3, color: "#78716c" }}>NO NEW IR · RANDOM-ACCESS SAFE · PATH-LOCAL</Text>
      </View>
    </Scene>
  );
}
