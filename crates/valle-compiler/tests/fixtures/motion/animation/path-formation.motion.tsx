export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "path-formation";

export const controls = defineControls({
  timing: { enterDuration: 0, exitDuration: 0 },
});

const agents = defineRepeater({ count: 26, keyPrefix: "agent" });
const ROUTE = path("M 180 750 C 315 180 750 195 885 525 C 1020 855 1455 870 1740 285");

export default function PathFormation(ctx) {
  const t = ctx.hold.progress;
  const draw = interpolate(t, [0, 0.28], [0, 1], { easing: "easeOut" });
  const labels = interpolate(t, [0.18, 0.4, 0.9, 1], [0, 1, 1, 0]);
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#070b13" }}>
      <Text key="kicker" className="absolute" style={{ left: 108, top: 81, fontSize: 24, letterSpacing: 6, color: "#34d399", opacity: labels }}>FORMATION STUDY / 02</Text>
      <Text key="title" className="absolute" style={{ left: 102, top: 130.5, width: 1350, fontSize: 73.5, color: "#f8fafc", opacity: labels }}>One path truth. Twenty-six followers.</Text>
      <Path key="route-shadow" d={ROUTE} fill="none" stroke="#172033" strokeWidth="24" strokeLinecap="round" />
      <Path key="route" d={ROUTE} fill="none" stroke={linearGradient(point(180, 750), point(1740, 285), [gradientStop(0, "#34d399"), gradientStop(0.5, "#22d3ee"), gradientStop(1, "#818cf8")])} strokeWidth="7.5" strokeLinecap="round" trimEnd={draw} arrowEnd="triangle" arrowSize="24" />
      {agents.map((agent) => (
        <View key={agent.key} className="absolute" style={{
          width: 12 + (1 - agent.progress) * 18,
          height: 12 + (1 - agent.progress) * 18,
          borderRadius: 18,
          backgroundColor: agent.index % 3 === 0 ? "#f8fafc" : agent.index % 3 === 1 ? "#67e8f9" : "#a7f3d0",
          opacity: 0.18 + (1 - agent.progress) * 0.82,
          filter: "drop-shadow(0px 0px 12px #67e8f9)",
          motionPath: follow(ROUTE, trail(t, agent.index, { gap: 0.031, mode: "wrap" }), { rotate: "auto" }),
        }} />
      ))}
      <View key="legend" className="absolute flex items-center" style={{ left: 108, top: 948, width: 750, height: 63, opacity: labels }}>
        <View key="legend-dot" style={{ width: 18, height: 18, borderRadius: 9, backgroundColor: "#67e8f9" }} />
        <Text key="legend-text" style={{ marginLeft: 21, fontSize: 25.5, color: "#94a3b8" }}>arc-length sampling · tangent rotation · wrapped trail</Text>
      </View>
    </Scene>
  );
}
