export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "project-font-brand";

export const controls = defineControls({
  timing: { enterDuration: 0, exitDuration: 0 },
  assets: {
    brandFont: asset({ kind: "font", required: true }),
  },
});

const intro = defineSequence({
  mark: stage({ duration: seconds(1.1) }),
  name: stage({ after: "mark", overlap: seconds(0.35), duration: seconds(1.25) }),
  line: stage({ after: "name", overlap: seconds(0.45), duration: seconds(1.1) }),
  settle: stage({ after: "line", duration: seconds(2.5) }),
});

const ORBIT = path("M 960 195 C 1353 195 1665 264 1665 540 C 1665 816 1353 945 960 945 C 567 945 255 816 255 540 C 255 264 567 195 960 195 Z");

export default function ProjectFontBrand(ctx) {
  const mark = stageProgress(ctx.localFrame, ctx.fps, intro.mark);
  const name = stageProgress(ctx.localFrame, ctx.fps, intro.name);
  const line = stageProgress(ctx.localFrame, ctx.fps, intro.line);
  const breathe = yoyoProgress(ctx.localFrame, ctx.fps, intro.settle, 3);
  const orbit = repeatProgress(ctx.localFrame, ctx.fps, intro.settle, 2);

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#05070c", fontFamily: "asset://brandFont" }}>
      <View key="aura-blue" className="absolute" style={{ left: 417, top: 48, width: 1086, height: 975, borderRadius: 540, backgroundColor: "#2563eb", opacity: 0.1 + breathe * 0.08, filter: "blur(168px)" }} />
      <View key="aura-cyan" className="absolute" style={{ left: 741, top: 285, width: 435, height: 435, borderRadius: 225, backgroundColor: "#22d3ee", opacity: 0.18, filter: "blur(132px)" }} />

      <Path key="orbit-base" d={ORBIT} fill="none" stroke="#172033" strokeWidth="1.5" strokeDasharray="5 13" />
      <Path key="orbit-live" d={ORBIT} fill="none" stroke={linearGradient(point(255, 540), point(1665, 540), [gradientStop(0, "#3b82f6"), gradientStop(0.5, "#67e8f9"), gradientStop(1, "#8b5cf6")])} strokeWidth="4.5" strokeLinecap="round" trimEnd={line} />
      <View key="orbit-light" className="absolute" style={{ width: 18, height: 18, borderRadius: 12, backgroundColor: "#ffffff", opacity: line, filter: "drop-shadow(0px 0px 21px #67e8f9)", motionPath: motionPath(ORBIT, orbit) }} />

      <View key="mark-shell" className="absolute" style={{ borderStyle: "solid", left: 837, top: 261, width: 246, height: 246, borderRadius: 69, borderWidth: 3, borderColor: "#67e8f966", backgroundColor: "#0b1220dd", opacity: mark, transform: `rotate(${(1 - mark) * -18}deg) scale(${0.7 + mark * 0.3})`, filter: "drop-shadow(0px 30px 63px #000000aa)" }}>
        <View key="mark-a" className="absolute" style={{ left: 51, top: 52.5, width: 66, height: 141, borderRadius: 33, backgroundColor: "#67e8f9", transform: `rotate(${18 - mark * 18}deg)`, filter: "drop-shadow(0px 0px 27px #22d3ee)" }} />
        <View key="mark-b" className="absolute" style={{ right: 51, top: 52.5, width: 66, height: 141, borderRadius: 33, backgroundColor: "#8b5cf6", transform: `rotate(${-18 + mark * 18}deg)`, filter: "drop-shadow(0px 0px 27px #8b5cf6)" }} />
      </View>

      <Text key="brand" className="absolute" style={{ left: 0, top: 540, width: 1920, textAlign: "center", fontFamily: "asset://brandFont", fontSize: 156, letterSpacing: 6, color: "#f8fafc", opacity: name, translate: point(0, (1 - name) * 63), textShadow: "0px 18px 51px #000000aa" }}>VALLE</Text>
      <Text key="claim" className="absolute" style={{ left: 0, top: 738, width: 1920, textAlign: "center", fontFamily: "asset://brandFont", fontSize: 37.5, letterSpacing: 3, color: "#94a3b8", opacity: line }}>
        {"DESIGNED FOR "}<Span style={{ color: "#67e8f9", fontWeight: 700 }}>MOTION</Span>{" · BUILT FOR EVERY FRAME"}
      </Text>
      <View key="rule" className="absolute" style={{ left: 735, top: 825, width: 450 * line, height: 3, backgroundColor: "#67e8f9", opacity: line, filter: "drop-shadow(0px 0px 12px #22d3ee)" }} />
    </Scene>
  );
}
