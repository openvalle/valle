export const component = "vector-turbulence-proxy";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const ribbons = defineRepeater({ count: 19, keyPrefix: "ribbon" });
const riders = defineRepeater({ count: 24, keyPrefix: "rider" });
const FLOW_A = path("M 90 372 C 230 132 414 590 576 342 C 742 90 902 572 1190 312");
const FLOW_B = path("M 90 320 C 254 574 404 86 602 390 C 770 648 948 118 1190 384");

export default function VectorTurbulenceProxy(ctx) {
  const t = ctx.hold.progress;
  const phase = (sin(t * 12.566370614 - 1.2) + 1) * 0.5;
  const flow = morphPath(FLOW_A, FLOW_B, phase);
  const reveal = interpolate(t, [0, 0.2], [0, 1], { easing: "easeOut" });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#f4f1e8" }}>
      <View key="ink-wash" className="absolute" style={{ left: 145, top: 165, width: 990, height: 420, borderRadius: 210, backgroundColor: "#fb718522", filter: "blur(68px)" }} />
      <Text key="kicker" className="absolute" style={{ left: 58, top: 43, fontSize: 15, letterSpacing: 4, color: "#dc2626", opacity: reveal }}>VECTOR DEFORMATION</Text>
      <Text key="title" className="absolute" style={{ left: 54, top: 78, width: 1120, fontSize: 52, color: "#171717", opacity: reveal }}>Turbulence, without a raster effect.</Text>
      <Text key="note" className="absolute" style={{ left: 58, top: 143, width: 900, fontSize: 18, color: "#57534e", opacity: reveal }}>A fixed-topology path morph drives nineteen deterministic offsets.</Text>

      {ribbons.map((ribbon) => (
        <Path key={ribbon.key} d={offsetPath(flow, (ribbon.index - 9) * 14)} fill="none" stroke={ribbon.index % 3 === 0 ? "#111827" : ribbon.index % 3 === 1 ? "#ef4444" : "#2563eb"} strokeWidth={ribbon.index === 9 ? 4 : 1.4} strokeLinecap="round" style={{ opacity: 0.12 + (1 - ribbon.progress) * 0.58 }} />
      ))}

      {riders.map((rider) => (
        <View key={rider.key} className="absolute" style={{ width: 5 + (1 - rider.progress) * 8, height: 5 + (1 - rider.progress) * 8, borderRadius: 8, backgroundColor: rider.index % 2 === 0 ? "#111827" : "#ef4444", opacity: 0.22 + (1 - rider.progress) * 0.72, filter: "drop-shadow(0px 0px 6px #ffffff)", motionPath: follow(flow, trail(t * 1.35, rider.index, { gap: 0.035, mode: "wrap" }), { rotate: "auto" }) }} />
      ))}

      <View key="legend" className="absolute flex items-center" style={{ borderStyle: "solid", left: 58, top: 626, width: 1164, height: 54, borderTopWidth: 1, borderColor: "#a8a29e" }}>
        <Text key="legend-a" style={{ fontSize: 14, letterSpacing: 2, color: "#171717" }}>MORPHPATH</Text>
        <Text key="legend-plus" style={{ marginLeft: 18, fontSize: 18, color: "#dc2626" }}>+</Text>
        <Text key="legend-b" style={{ marginLeft: 18, fontSize: 14, letterSpacing: 2, color: "#171717" }}>OFFSETPATH</Text>
        <Text key="legend-result" style={{ marginLeft: 42, fontSize: 14, letterSpacing: 2, color: "#78716c" }}>NO NEW IR · RANDOM-ACCESS SAFE · PATH-LOCAL</Text>
      </View>
    </Scene>
  );
}
