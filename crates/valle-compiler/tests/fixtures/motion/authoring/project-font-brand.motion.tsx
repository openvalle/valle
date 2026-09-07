export const component = "project-font-brand";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
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

const ORBIT = path("M 640 130 C 902 130 1110 176 1110 360 C 1110 544 902 630 640 630 C 378 630 170 544 170 360 C 170 176 378 130 640 130 Z");

export default function ProjectFontBrand(ctx) {
  const mark = stageProgress(ctx.localFrame, ctx.fps, intro.mark);
  const name = stageProgress(ctx.localFrame, ctx.fps, intro.name);
  const line = stageProgress(ctx.localFrame, ctx.fps, intro.line);
  const breathe = yoyoProgress(ctx.localFrame, ctx.fps, intro.settle, 3);
  const orbit = repeatProgress(ctx.localFrame, ctx.fps, intro.settle, 2);

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#05070c", fontFamily: "asset://brandFont" }}>
      <View key="aura-blue" className="absolute" style={{ left: 278, top: 32, width: 724, height: 650, borderRadius: 360, backgroundColor: "#2563eb", opacity: 0.1 + breathe * 0.08, filter: "blur(112px)" }} />
      <View key="aura-cyan" className="absolute" style={{ left: 494, top: 190, width: 290, height: 290, borderRadius: 150, backgroundColor: "#22d3ee", opacity: 0.18, filter: "blur(88px)" }} />

      <Path key="orbit-base" d={ORBIT} fill="none" stroke="#172033" strokeWidth="1" strokeDasharray="5 13" />
      <Path key="orbit-live" d={ORBIT} fill="none" stroke={linearGradient(point(170, 360), point(1110, 360), [gradientStop(0, "#3b82f6"), gradientStop(0.5, "#67e8f9"), gradientStop(1, "#8b5cf6")])} strokeWidth="3" strokeLinecap="round" trimEnd={line} />
      <View key="orbit-light" className="absolute" style={{ width: 12, height: 12, borderRadius: 8, backgroundColor: "#ffffff", opacity: line, filter: "drop-shadow(0px 0px 14px #67e8f9)", motionPath: motionPath(ORBIT, orbit) }} />

      <View key="mark-shell" className="absolute" style={{ borderStyle: "solid", left: 558, top: 174, width: 164, height: 164, borderRadius: 46, borderWidth: 2, borderColor: "#67e8f966", backgroundColor: "#0b1220dd", opacity: mark, transform: `rotate(${(1 - mark) * -18}deg) scale(${0.7 + mark * 0.3})`, filter: "drop-shadow(0px 20px 42px #000000aa)" }}>
        <View key="mark-a" className="absolute" style={{ left: 34, top: 35, width: 44, height: 94, borderRadius: 22, backgroundColor: "#67e8f9", transform: `rotate(${18 - mark * 18}deg)`, filter: "drop-shadow(0px 0px 18px #22d3ee)" }} />
        <View key="mark-b" className="absolute" style={{ right: 34, top: 35, width: 44, height: 94, borderRadius: 22, backgroundColor: "#8b5cf6", transform: `rotate(${-18 + mark * 18}deg)`, filter: "drop-shadow(0px 0px 18px #8b5cf6)" }} />
      </View>

      <Text key="brand" className="absolute" style={{ left: 0, top: 360, width: 1280, textAlign: "center", fontFamily: "asset://brandFont", fontSize: 104, letterSpacing: 4, color: "#f8fafc", opacity: name, translate: point(0, (1 - name) * 42), textShadow: "0px 12px 34px #000000aa" }}>VALLE</Text>
      <Text key="claim" className="absolute" style={{ left: 0, top: 492, width: 1280, textAlign: "center", fontFamily: "asset://brandFont", fontSize: 25, letterSpacing: 2, color: "#94a3b8", opacity: line }}>
        {"DESIGNED FOR "}<Span style={{ color: "#67e8f9", fontWeight: 700 }}>MOTION</Span>{" · BUILT FOR EVERY FRAME"}
      </Text>
      <View key="rule" className="absolute" style={{ left: 490, top: 550, width: 300 * line, height: 2, backgroundColor: "#67e8f9", opacity: line, filter: "drop-shadow(0px 0px 8px #22d3ee)" }} />
    </Scene>
  );
}
