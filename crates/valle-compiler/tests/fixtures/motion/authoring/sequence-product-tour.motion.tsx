export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "sequence-product-tour";

export const controls = defineControls({
  timing: { enterDuration: 0, exitDuration: 0 },
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
      <View key="back-glow" className="absolute" style={{ left: 1102.5, top: 45, width: 780, height: 930, borderRadius: 420, backgroundColor: "#7c3aed", opacity: 0.12 + pulse * 0.05, filter: "blur(157.5px)" }} />
      <Text key="eyebrow" className="absolute" style={{ left: 96, top: 57, fontSize: 25.5, letterSpacing: 3, color: "#a78bfa", opacity: chrome }}>VALLE PRODUCT TOUR / 04</Text>
      <Text key="headline" className="absolute" style={{ left: 93, top: 112.5, width: 1680, fontSize: 78, color: "#f8fafc", opacity: chrome, translate: point(0, (1 - chrome) * 36) }}>One sequence. Clear intent.</Text>
      <Text key="subhead" className="absolute" style={{ left: 97.5, top: 225, width: 930, fontSize: 31.5, color: "#64748b", opacity: chrome }}>Named stages compile into deterministic frame math.</Text>

      <View key="app" className="absolute" style={{ borderStyle: "solid", left: 93, top: 315, width: 1734, height: 675, borderRadius: 39, backgroundColor: "#101621", borderWidth: 1.5, borderColor: "#273247", opacity: chrome, transform: `scale(${0.96 + chrome * 0.04})`, filter: "drop-shadow(0px 48px 90px #00000099)" }}>
        <View key="topbar" className="absolute flex items-center" style={{ borderStyle: "solid", left: 0, top: 0, width: 1734, height: 93, borderBottomWidth: 1.5, borderColor: "#273247" }}>
          <View key="dot-r" style={{ marginLeft: 33, width: 16.5, height: 16.5, borderRadius: 9, backgroundColor: "#fb7185" }} />
          <View key="dot-y" style={{ marginLeft: 12, width: 16.5, height: 16.5, borderRadius: 9, backgroundColor: "#fbbf24" }} />
          <View key="dot-g" style={{ marginLeft: 12, width: 16.5, height: 16.5, borderRadius: 9, backgroundColor: "#34d399" }} />
          <Text key="app-title" style={{ marginLeft: 592.5, fontSize: 25.5, color: "#94a3b8" }}>Launch workspace</Text>
        </View>

        <View key="sidebar" className="absolute" style={{ borderStyle: "solid", left: 0, top: 93, width: 327, height: 582, borderRightWidth: 1, borderColor: "#273247", opacity: nav }}>
          {STEPS.map((label, i) => (
            <View key={`nav-${i}`} className="absolute flex items-center" style={{ left: 27, top: 36 + i * 108, width: 273, height: 78, borderRadius: 21, backgroundColor: i === 1 ? "#6d28d933" : "#00000000", opacity: staggerProgress(ctx.localFrame, ctx.fps, tour.navigation, i, seconds(0.1)), translate: point((1 - staggerProgress(ctx.localFrame, ctx.fps, tour.navigation, i, seconds(0.1))) * -36, 0) }}>
              <View key={`nav-icon-${i}`} style={{ marginLeft: 19.5, width: 39, height: 39, borderRadius: 13.5, backgroundColor: i === 1 ? "#8b5cf6" : "#263247" }} />
              <Text key={`nav-label-${i}`} style={{ marginLeft: 18, fontSize: 25.5, color: i === 1 ? "#ede9fe" : "#718096" }}>{label}</Text>
            </View>
          ))}
        </View>

        <View key="workspace" className="absolute" style={{ borderStyle: "solid", left: 366, top: 129, width: 825, height: 501, borderRadius: 30, backgroundColor: "#0c111a", borderWidth: 1.5, borderColor: "#222e41", opacity: work, translate: point(0, (1 - work) * 39) }}>
          <Text key="workspace-label" className="absolute" style={{ left: 36, top: 33, fontSize: 22.5, letterSpacing: 1.5, color: "#64748b" }}>SEQUENCE GRAPH</Text>
          {STEPS.map((label, i) => (
            <View key={`stage-${i}`} className="absolute" style={{ borderStyle: "solid", left: 37.5 + i * 258, top: 129, width: 222, height: 174, borderRadius: 27, backgroundColor: i === 1 ? "#6d28d944" : "#182131", borderWidth: 1.5, borderColor: i === 1 ? "#a78bfa" : "#314057", opacity: staggerProgress(ctx.localFrame, ctx.fps, tour.workspace, i, seconds(0.12)), transform: `scale(${0.82 + staggerProgress(ctx.localFrame, ctx.fps, tour.workspace, i, seconds(0.12)) * 0.18})` }}>
              <Text key={`stage-num-${i}`} className="absolute" style={{ left: 24, top: 19.5, fontSize: 21, color: "#8b9ab0" }}>{padNumber(i + 1, { width: 2 })}</Text>
              <Text key={`stage-name-${i}`} className="absolute" style={{ left: 24, top: 73.5, fontSize: 28.5, color: "#f1f5f9" }}>{label}</Text>
            </View>
          ))}
          <View key="timeline-track" className="absolute" style={{ left: 37.5, top: 378, width: 750, height: 12, borderRadius: 6, backgroundColor: "#202b3d" }} />
          <View key="timeline-fill" className="absolute" style={{ left: 37.5, top: 378, width: 750 * done, height: 12, borderRadius: 6, backgroundColor: "#8b5cf6", filter: "drop-shadow(0px 0px 15px #8b5cf6)" }} />
        </View>

        <View key="insights" className="absolute" style={{ borderStyle: "solid", right: 36, top: 129, width: 465, height: 501, borderRadius: 30, backgroundColor: "#0c111a", borderWidth: 1.5, borderColor: "#222e41", opacity: insights, translate: point((1 - insights) * 48, 0) }}>
          <Text key="insight-label" className="absolute" style={{ left: 33, top: 31.5, fontSize: 22.5, color: "#64748b" }}>FRAME CONFIDENCE</Text>
          <Text key="insight-value" className="absolute" style={{ left: 30, top: 75, fontSize: 93, color: "#f8fafc" }}>{formatPercent(0.999, { decimals: 1 })}</Text>
          {VALUES.map((value, i) => (
            <View key={`mini-${i}`} className="absolute" style={{ left: 36 + i * 64.5, bottom: 52.5, width: 37.5, height: insights * value * 2.175, borderRadius: 12, backgroundColor: i === 5 ? "#a78bfa" : "#334155" }} />
          ))}
          <Text key="done" className="absolute" style={{ left: 33, top: 225, fontSize: 27, color: "#6ee7b7", opacity: done }}>● READY TO PUBLISH</Text>
        </View>
      </View>
    </Scene>
  );
}
