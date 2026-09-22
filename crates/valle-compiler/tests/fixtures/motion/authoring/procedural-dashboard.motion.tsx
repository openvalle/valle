export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

const BARS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const GRID = [0, 1, 2, 3, 4, 5];
const RING = path("M 307.5 217.5 C 438 217.5 540 319.5 540 450 C 540 580.5 438 682.5 307.5 682.5 C 177 682.5 75 580.5 75 450 C 75 319.5 177 217.5 307.5 217.5 Z");

export default function ProceduralDashboard(ctx) {
  const t = ctx.progress;
  const wave = (sin(t * 6.283185307 + 0.4) + 1) * 0.5;
  const health = 0.91 + sin(t * 12.566370614) * 0.035;
  const orbit = fract(t * 1.6);
  const value = 48210 + round(wave * 7350);

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#03080d" }}>
      <View key="mesh" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080, backgroundColor: "#07111a" }} />
      <View key="cyan-haze" className="absolute" style={{ left: 180, top: 270, width: 810, height: 690, borderRadius: 420, backgroundColor: "#06b6d4", opacity: 0.08 + noise1d(31, t * 4) * 0.035, filter: "blur(150px)" }} />

      <Text key="system" className="absolute" style={{ left: 78, top: 54, fontSize: 27, letterSpacing: 3, color: "#22d3ee" }}>PROCEDURAL SIGNAL / LIVE</Text>
      <Text key="clock" className="absolute" style={{ right: 81, top: 51, fontSize: 30, color: "#64748b" }}>{padNumber(round(t * 359), { width: 3 })}{" / 359 FRAMES"}</Text>

      <View key="hero-card" className="absolute" style={{ borderStyle: "solid", left: 75, top: 138, width: 1113, height: 357, borderRadius: 39, backgroundColor: "#09131dcc", borderWidth: 1.5, borderColor: "#164e63", filter: "drop-shadow(0px 36px 81px #00000088)" }}>
        <Text key="metric-label" className="absolute" style={{ left: 42, top: 36, fontSize: 24, color: "#67e8f9" }}>RENDERED EVENTS</Text>
        <Text key="metric-number" className="absolute" style={{ left: 36, top: 82.5, fontSize: 117, color: "#f8fafc" }}>{formatNumber(value, { decimals: 0, grouping: true })}</Text>
        <Text key="metric-delta" className="absolute" style={{ left: 45, top: 232.5, fontSize: 28.5, color: "#6ee7b7" }}>{"+ "}{formatPercent(0.084 + wave * 0.018, { decimals: 1 })}{" vs previous cycle"}</Text>
        {GRID.map((line, i) => (
          <View key={`grid-${i}`} className="absolute" style={{ left: 495, top: 52.5 + i * 45, width: 567, height: 1.5, backgroundColor: "#173042", opacity: 0.6 }} />
        ))}
        {BARS.map((i) => (
          <View key={`hero-bar-${i}`} className="absolute" style={{ left: 516 + i * 43.5, bottom: 51, width: 22.5, height: 54 + noise2d(71, i * 0.37, t * 5.5) * 168 + (sin(t * 8 + i * 0.7) + 1) * 24, borderRadius: 12, backgroundColor: i > 8 ? "#a78bfa" : "#22d3ee", opacity: 0.5 + noise2d(71, i * 0.37, t * 5.5) * 0.5, filter: "drop-shadow(0px 0px 12px #22d3ee)" }} />
        ))}
      </View>

      <View key="latency" className="absolute" style={{ borderStyle: "solid", left: 75, top: 531, width: 540, height: 465, borderRadius: 37.5, backgroundColor: "#09131dcc", borderWidth: 1.5, borderColor: "#1e293b" }}>
        <Text key="latency-label" className="absolute" style={{ left: 39, top: 34.5, fontSize: 24, color: "#94a3b8" }}>FRAME LATENCY</Text>
        <Text key="latency-value" className="absolute" style={{ left: 36, top: 85.5, fontSize: 90, color: "#ffffff" }}>{formatNumber(8.2 + wave * 1.6, { decimals: 1 })}</Text>
        <Text key="latency-unit" className="absolute" style={{ left: 315, top: 136.5, fontSize: 25.5, color: "#64748b" }}>MS</Text>
        {BARS.map((i) => (
          <View key={`latency-bar-${i}`} className="absolute" style={{ left: 40.5 + i * 37.5, bottom: 48, width: 19.5, height: 52.5 + noise1d(19, t * 7 + i * 0.6) * 142.5, borderRadius: 10.5, backgroundColor: "#0ea5e9", opacity: 0.45 + i * 0.035 }} />
        ))}
      </View>

      <View key="throughput" className="absolute" style={{ borderStyle: "solid", left: 651, top: 531, width: 537, height: 465, borderRadius: 37.5, backgroundColor: "#09131dcc", borderWidth: 1.5, borderColor: "#1e293b" }}>
        <Text key="throughput-label" className="absolute" style={{ left: 39, top: 34.5, fontSize: 24, color: "#94a3b8" }}>THROUGHPUT</Text>
        <Text key="throughput-value" className="absolute" style={{ left: 36, top: 85.5, fontSize: 90, color: "#ffffff" }}>{formatNumber(2.8 + wave * 0.5, { decimals: 2 })}</Text>
        <Text key="throughput-unit" className="absolute" style={{ left: 321, top: 136.5, fontSize: 25.5, color: "#64748b" }}>GB/S</Text>
        <View key="wave-track" className="absolute" style={{ left: 42, bottom: 87, width: 450, height: 6, borderRadius: 3, backgroundColor: "#1e293b" }} />
        <View key="wave-fill" className="absolute" style={{ left: 42, bottom: 87, width: 450 * wave, height: 6, borderRadius: 3, backgroundColor: "#8b5cf6", filter: "drop-shadow(0px 0px 15px #8b5cf6)" }} />
        <View key="wave-dot" className="absolute" style={{ left: 34.5 + 450 * wave, bottom: 76.5, width: 27, height: 27, borderRadius: 15, backgroundColor: "#ffffff", filter: "drop-shadow(0px 0px 18px #8b5cf6)" }} />
      </View>

      <View key="health-card" className="absolute" style={{ borderStyle: "solid", right: 75, top: 138, width: 615, height: 858, borderRadius: 42, backgroundColor: "#09131dcc", borderWidth: 1.5, borderColor: "#312e81" }}>
        <Text key="health-label" className="absolute" style={{ left: 45, top: 39, fontSize: 24, color: "#a5b4fc" }}>SYSTEM HEALTH</Text>
        <Path key="ring-bg" d={RING} fill="none" stroke="#172033" strokeWidth="27" />
        <Path key="ring-live" d={RING} fill="none" stroke={linearGradient(point(75, 450), point(540, 450), [gradientStop(0, "#22d3ee"), gradientStop(1, "#8b5cf6")])} strokeWidth="27" strokeLinecap="round" trimEnd={health} />
        <View key="ring-dot" className="absolute" style={{ width: 21, height: 21, borderRadius: 12, backgroundColor: "#ffffff", filter: "drop-shadow(0px 0px 21px #67e8f9)", motionPath: motionPath(RING, orbit) }} />
        <Text key="health-value" className="absolute" style={{ left: 0, top: 390, width: 615, textAlign: "center", fontSize: 100.5, color: "#ffffff" }}>{formatPercent(health, { decimals: 1 })}</Text>
        <Text key="health-copy" className="absolute" style={{ left: 30, top: 690, width: 555, textAlign: "center", fontSize: 22.5, color: "#64748b" }}>ALL DETERMINISTIC GATES NOMINAL</Text>
        <Text key="seed" className="absolute" style={{ left: 46.5, bottom: 42, fontSize: 22.5, color: "#475569" }}>SEED 0071 / NOISE FIELD LOCKED</Text>
      </View>
    </Scene>
  );
}
