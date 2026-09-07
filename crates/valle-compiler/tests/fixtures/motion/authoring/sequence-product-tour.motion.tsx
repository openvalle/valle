export const component = "sequence-product-tour";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const tour = defineSequence({
  chrome: stage({ duration: seconds(0.8) }),
  navigation: stage({ after: "chrome", overlap: seconds(0.25), duration: seconds(0.9) }),
  workspace: stage({ after: "navigation", overlap: seconds(0.2), duration: seconds(1.2) }),
  insights: stage({ after: "workspace", overlap: seconds(0.35), duration: seconds(1.15) }),
  complete: stage({ after: "insights", overlap: seconds(0.15), duration: seconds(1.2) }),
  pulse: stage({ after: "complete", overlap: seconds(0.8), duration: seconds(1.8) }),
});

const STEPS = ["Connect", "Shape", "Publish"];
const VALUES = [72, 91, 84, 96, 78, 100];

export default function SequenceProductTour(ctx) {
  const chrome = stageProgress(ctx.localFrame, ctx.fps, tour.chrome);
  const nav = stageProgress(ctx.localFrame, ctx.fps, tour.navigation);
  const work = stageProgress(ctx.localFrame, ctx.fps, tour.workspace);
  const insights = stageProgress(ctx.localFrame, ctx.fps, tour.insights);
  const done = stageProgress(ctx.localFrame, ctx.fps, tour.complete);
  const pulse = yoyoProgress(ctx.localFrame, ctx.fps, tour.pulse, 3);

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0a0d14" }}>
      <View key="back-glow" className="absolute" style={{ left: 735, top: 30, width: 520, height: 620, borderRadius: 280, backgroundColor: "#7c3aed", opacity: 0.12 + pulse * 0.05, filter: "blur(105px)" }} />
      <Text key="eyebrow" className="absolute" style={{ left: 64, top: 38, fontSize: 17, letterSpacing: 2, color: "#a78bfa", opacity: chrome }}>VALLE PRODUCT TOUR / 04</Text>
      <Text key="headline" className="absolute" style={{ left: 62, top: 75, width: 690, fontSize: 54, color: "#f8fafc", opacity: chrome, translate: point(0, (1 - chrome) * 24) }}>One sequence. Clear intent.</Text>
      <Text key="subhead" className="absolute" style={{ left: 65, top: 150, width: 620, fontSize: 21, color: "#64748b", opacity: chrome }}>Named stages compile into deterministic frame math.</Text>

      <View key="app" className="absolute" style={{ borderStyle: "solid", left: 62, top: 210, width: 1156, height: 450, borderRadius: 26, backgroundColor: "#101621", borderWidth: 1, borderColor: "#273247", opacity: chrome, transform: `scale(${0.96 + chrome * 0.04})`, filter: "drop-shadow(0px 32px 60px #00000099)" }}>
        <View key="topbar" className="absolute flex items-center" style={{ borderStyle: "solid", left: 0, top: 0, width: 1156, height: 62, borderBottomWidth: 1, borderColor: "#273247" }}>
          <View key="dot-r" style={{ marginLeft: 22, width: 11, height: 11, borderRadius: 6, backgroundColor: "#fb7185" }} />
          <View key="dot-y" style={{ marginLeft: 8, width: 11, height: 11, borderRadius: 6, backgroundColor: "#fbbf24" }} />
          <View key="dot-g" style={{ marginLeft: 8, width: 11, height: 11, borderRadius: 6, backgroundColor: "#34d399" }} />
          <Text key="app-title" style={{ marginLeft: 395, fontSize: 17, color: "#94a3b8" }}>Launch workspace</Text>
        </View>

        <View key="sidebar" className="absolute" style={{ borderStyle: "solid", left: 0, top: 62, width: 218, height: 388, borderRightWidth: 1, borderColor: "#273247", opacity: nav }}>
          {STEPS.map((label, i) => (
            <View key={`nav-${i}`} className="absolute flex items-center" style={{ left: 18, top: 24 + i * 72, width: 182, height: 52, borderRadius: 14, backgroundColor: i === 1 ? "#6d28d933" : "#00000000", opacity: staggerProgress(ctx.localFrame, ctx.fps, tour.navigation, i, seconds(0.1)), translate: point((1 - staggerProgress(ctx.localFrame, ctx.fps, tour.navigation, i, seconds(0.1))) * -24, 0) }}>
              <View key={`nav-icon-${i}`} style={{ marginLeft: 13, width: 26, height: 26, borderRadius: 9, backgroundColor: i === 1 ? "#8b5cf6" : "#263247" }} />
              <Text key={`nav-label-${i}`} style={{ marginLeft: 12, fontSize: 17, color: i === 1 ? "#ede9fe" : "#718096" }}>{label}</Text>
            </View>
          ))}
        </View>

        <View key="workspace" className="absolute" style={{ borderStyle: "solid", left: 244, top: 86, width: 550, height: 334, borderRadius: 20, backgroundColor: "#0c111a", borderWidth: 1, borderColor: "#222e41", opacity: work, translate: point(0, (1 - work) * 26) }}>
          <Text key="workspace-label" className="absolute" style={{ left: 24, top: 22, fontSize: 15, letterSpacing: 1, color: "#64748b" }}>SEQUENCE GRAPH</Text>
          {STEPS.map((label, i) => (
            <View key={`stage-${i}`} className="absolute" style={{ borderStyle: "solid", left: 25 + i * 171, top: 86, width: 148, height: 116, borderRadius: 18, backgroundColor: i === 1 ? "#6d28d944" : "#182131", borderWidth: 1, borderColor: i === 1 ? "#a78bfa" : "#314057", opacity: staggerProgress(ctx.localFrame, ctx.fps, tour.workspace, i, seconds(0.12)), transform: `scale(${0.82 + staggerProgress(ctx.localFrame, ctx.fps, tour.workspace, i, seconds(0.12)) * 0.18})` }}>
              <Text key={`stage-num-${i}`} className="absolute" style={{ left: 16, top: 13, fontSize: 14, color: "#8b9ab0" }}>{padNumber(i + 1, { width: 2 })}</Text>
              <Text key={`stage-name-${i}`} className="absolute" style={{ left: 16, top: 49, fontSize: 19, color: "#f1f5f9" }}>{label}</Text>
            </View>
          ))}
          <View key="timeline-track" className="absolute" style={{ left: 25, top: 252, width: 500, height: 8, borderRadius: 4, backgroundColor: "#202b3d" }} />
          <View key="timeline-fill" className="absolute" style={{ left: 25, top: 252, width: 500 * done, height: 8, borderRadius: 4, backgroundColor: "#8b5cf6", filter: "drop-shadow(0px 0px 10px #8b5cf6)" }} />
        </View>

        <View key="insights" className="absolute" style={{ borderStyle: "solid", right: 24, top: 86, width: 310, height: 334, borderRadius: 20, backgroundColor: "#0c111a", borderWidth: 1, borderColor: "#222e41", opacity: insights, translate: point((1 - insights) * 32, 0) }}>
          <Text key="insight-label" className="absolute" style={{ left: 22, top: 21, fontSize: 15, color: "#64748b" }}>FRAME CONFIDENCE</Text>
          <Text key="insight-value" className="absolute" style={{ left: 20, top: 50, fontSize: 62, color: "#f8fafc" }}>{formatPercent(0.999, { decimals: 1 })}</Text>
          {VALUES.map((value, i) => (
            <View key={`mini-${i}`} className="absolute" style={{ left: 24 + i * 43, bottom: 35, width: 25, height: insights * value * 1.45, borderRadius: 8, backgroundColor: i === 5 ? "#a78bfa" : "#334155" }} />
          ))}
          <Text key="done" className="absolute" style={{ left: 22, top: 150, fontSize: 18, color: "#6ee7b7", opacity: done }}>● READY TO PUBLISH</Text>
        </View>
      </View>
    </Scene>
  );
}
