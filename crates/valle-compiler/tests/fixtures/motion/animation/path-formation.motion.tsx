export const component = "path-formation";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const agents = defineRepeater({ count: 26, keyPrefix: "agent" });
const ROUTE = path("M 120 500 C 210 120 500 130 590 350 C 680 570 970 580 1160 190");

export default function PathFormation(ctx) {
  const t = ctx.hold.progress;
  const draw = interpolate(t, [0, 0.28], [0, 1], { easing: "easeOut" });
  const labels = interpolate(t, [0.18, 0.4, 0.9, 1], [0, 1, 1, 0]);
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#070b13" }}>
      <Text key="kicker" className="absolute" style={{ left: 72, top: 54, fontSize: 16, letterSpacing: 4, color: "#34d399", opacity: labels }}>FORMATION STUDY / 02</Text>
      <Text key="title" className="absolute" style={{ left: 68, top: 87, width: 900, fontSize: 49, color: "#f8fafc", opacity: labels }}>One path truth. Twenty-six followers.</Text>
      <Path key="route-shadow" d={ROUTE} fill="none" stroke="#172033" strokeWidth="16" strokeLinecap="round" />
      <Path key="route" d={ROUTE} fill="none" stroke={linearGradient(point(120, 500), point(1160, 190), [gradientStop(0, "#34d399"), gradientStop(0.5, "#22d3ee"), gradientStop(1, "#818cf8")])} strokeWidth="5" strokeLinecap="round" trimEnd={draw} arrowEnd="triangle" arrowSize="16" />
      {agents.map((agent) => (
        <View key={agent.key} className="absolute" style={{
          width: 8 + (1 - agent.progress) * 12,
          height: 8 + (1 - agent.progress) * 12,
          borderRadius: 12,
          backgroundColor: agent.index % 3 === 0 ? "#f8fafc" : agent.index % 3 === 1 ? "#67e8f9" : "#a7f3d0",
          opacity: 0.18 + (1 - agent.progress) * 0.82,
          filter: "drop-shadow(0px 0px 8px #67e8f9)",
          motionPath: follow(ROUTE, trail(t, agent.index, { gap: 0.031, mode: "wrap" }), { rotate: "auto" }),
        }} />
      ))}
      <View key="legend" className="absolute flex items-center" style={{ left: 72, top: 632, width: 500, height: 42, opacity: labels }}>
        <View key="legend-dot" style={{ width: 12, height: 12, borderRadius: 6, backgroundColor: "#67e8f9" }} />
        <Text key="legend-text" style={{ marginLeft: 14, fontSize: 17, color: "#94a3b8" }}>arc-length sampling · tangent rotation · wrapped trail</Text>
      </View>
    </Scene>
  );
}
