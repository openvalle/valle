export const component = "procedural-dashboard";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const BARS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const GRID = [0, 1, 2, 3, 4, 5];
const RING = path("M 1030 218 C 1124 218 1198 292 1198 386 C 1198 480 1124 554 1030 554 C 936 554 862 480 862 386 C 862 292 936 218 1030 218 Z");

export default function ProceduralDashboard(ctx) {
  const t = ctx.hold.progress;
  const wave = (sin(t * 6.283185307 + 0.4) + 1) * 0.5;
  const health = 0.91 + sin(t * 12.566370614) * 0.035;
  const orbit = fract(t * 1.6);
  const value = 48210 + round(wave * 7350);

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#03080d" }}>
      <View key="mesh" className="absolute" style={{ left: 0, top: 0, width: 1280, height: 720, backgroundColor: "#07111a" }} />
      <View key="cyan-haze" className="absolute" style={{ left: 120, top: 180, width: 540, height: 460, borderRadius: 280, backgroundColor: "#06b6d4", opacity: 0.08 + noise1d(31, t * 4) * 0.035, filter: "blur(100px)" }} />

      <Text key="system" className="absolute" style={{ left: 52, top: 36, fontSize: 18, letterSpacing: 2, color: "#22d3ee" }}>PROCEDURAL SIGNAL / LIVE</Text>
      <Text key="clock" className="absolute" style={{ right: 54, top: 34, fontSize: 20, color: "#64748b" }}>{padNumber(round(t * 359), { width: 3 })}{" / 359 FRAMES"}</Text>

      <View key="hero-card" className="absolute" style={{ borderStyle: "solid", left: 50, top: 92, width: 742, height: 238, borderRadius: 26, backgroundColor: "#09131dcc", borderWidth: 1, borderColor: "#164e63", filter: "drop-shadow(0px 24px 54px #00000088)" }}>
        <Text key="metric-label" className="absolute" style={{ left: 28, top: 24, fontSize: 16, color: "#67e8f9" }}>RENDERED EVENTS</Text>
        <Text key="metric-number" className="absolute" style={{ left: 24, top: 55, fontSize: 78, color: "#f8fafc" }}>{formatNumber(value, { decimals: 0, grouping: true })}</Text>
        <Text key="metric-delta" className="absolute" style={{ left: 30, top: 155, fontSize: 19, color: "#6ee7b7" }}>{"+ "}{formatPercent(0.084 + wave * 0.018, { decimals: 1 })}{" vs previous cycle"}</Text>
        {GRID.map((line, i) => (
          <View key={`grid-${i}`} className="absolute" style={{ left: 330, top: 35 + i * 30, width: 378, height: 1, backgroundColor: "#173042", opacity: 0.6 }} />
        ))}
        {BARS.map((i) => (
          <View key={`hero-bar-${i}`} className="absolute" style={{ left: 344 + i * 29, bottom: 34, width: 15, height: 36 + noise2d(71, i * 0.37, t * 5.5) * 112 + (sin(t * 8 + i * 0.7) + 1) * 16, borderRadius: 8, backgroundColor: i > 8 ? "#a78bfa" : "#22d3ee", opacity: 0.5 + noise2d(71, i * 0.37, t * 5.5) * 0.5, filter: "drop-shadow(0px 0px 8px #22d3ee)" }} />
        ))}
      </View>

      <View key="latency" className="absolute" style={{ borderStyle: "solid", left: 50, top: 354, width: 360, height: 310, borderRadius: 25, backgroundColor: "#09131dcc", borderWidth: 1, borderColor: "#1e293b" }}>
        <Text key="latency-label" className="absolute" style={{ left: 26, top: 23, fontSize: 16, color: "#94a3b8" }}>FRAME LATENCY</Text>
        <Text key="latency-value" className="absolute" style={{ left: 24, top: 57, fontSize: 60, color: "#ffffff" }}>{formatNumber(8.2 + wave * 1.6, { decimals: 1 })}</Text>
        <Text key="latency-unit" className="absolute" style={{ left: 210, top: 91, fontSize: 17, color: "#64748b" }}>MS</Text>
        {BARS.map((i) => (
          <View key={`latency-bar-${i}`} className="absolute" style={{ left: 27 + i * 25, bottom: 32, width: 13, height: 35 + noise1d(19, t * 7 + i * 0.6) * 95, borderRadius: 7, backgroundColor: "#0ea5e9", opacity: 0.45 + i * 0.035 }} />
        ))}
      </View>

      <View key="throughput" className="absolute" style={{ borderStyle: "solid", left: 434, top: 354, width: 358, height: 310, borderRadius: 25, backgroundColor: "#09131dcc", borderWidth: 1, borderColor: "#1e293b" }}>
        <Text key="throughput-label" className="absolute" style={{ left: 26, top: 23, fontSize: 16, color: "#94a3b8" }}>THROUGHPUT</Text>
        <Text key="throughput-value" className="absolute" style={{ left: 24, top: 57, fontSize: 60, color: "#ffffff" }}>{formatNumber(2.8 + wave * 0.5, { decimals: 2 })}</Text>
        <Text key="throughput-unit" className="absolute" style={{ left: 214, top: 91, fontSize: 17, color: "#64748b" }}>GB/S</Text>
        <View key="wave-track" className="absolute" style={{ left: 28, bottom: 58, width: 300, height: 4, borderRadius: 2, backgroundColor: "#1e293b" }} />
        <View key="wave-fill" className="absolute" style={{ left: 28, bottom: 58, width: 300 * wave, height: 4, borderRadius: 2, backgroundColor: "#8b5cf6", filter: "drop-shadow(0px 0px 10px #8b5cf6)" }} />
        <View key="wave-dot" className="absolute" style={{ left: 23 + 300 * wave, bottom: 51, width: 18, height: 18, borderRadius: 10, backgroundColor: "#ffffff", filter: "drop-shadow(0px 0px 12px #8b5cf6)" }} />
      </View>

      <View key="health-card" className="absolute" style={{ borderStyle: "solid", right: 50, top: 92, width: 410, height: 572, borderRadius: 28, backgroundColor: "#09131dcc", borderWidth: 1, borderColor: "#312e81" }}>
        <Text key="health-label" className="absolute" style={{ left: 30, top: 26, fontSize: 16, color: "#a5b4fc" }}>SYSTEM HEALTH</Text>
        <Path key="ring-bg" d={RING} fill="none" stroke="#172033" strokeWidth="18" />
        <Path key="ring-live" d={RING} fill="none" stroke={linearGradient(point(862, 386), point(1198, 386), [gradientStop(0, "#22d3ee"), gradientStop(1, "#8b5cf6")])} strokeWidth="18" strokeLinecap="round" trimEnd={health} />
        <View key="ring-dot" className="absolute" style={{ width: 14, height: 14, borderRadius: 8, backgroundColor: "#ffffff", filter: "drop-shadow(0px 0px 14px #67e8f9)", motionPath: motionPath(RING, orbit) }} />
        <Text key="health-value" className="absolute" style={{ left: 0, top: 260, width: 410, textAlign: "center", fontSize: 67, color: "#ffffff" }}>{formatPercent(health, { decimals: 1 })}</Text>
        <Text key="health-copy" className="absolute" style={{ left: 0, top: 342, width: 410, textAlign: "center", fontSize: 17, color: "#64748b" }}>ALL DETERMINISTIC GATES NOMINAL</Text>
        <Text key="seed" className="absolute" style={{ left: 31, bottom: 28, fontSize: 15, color: "#475569" }}>SEED 0071 / NOISE FIELD LOCKED</Text>
      </View>
    </Scene>
  );
}
