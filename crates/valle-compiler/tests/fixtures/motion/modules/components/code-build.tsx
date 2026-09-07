import { releaseTheme } from "./shared-theme";

export const component = "code-build";

export const controls = defineControls({
  props: { accentStrength: number({ default: 1, min: 0.25, max: 1 }) },
  assets: { dot: asset({ kind: "image" }) },
  cues: { narration: spanCue() },
});

const BUILD = defineSequence({
  author: stage({ duration: seconds(1.4) }),
  compile: stage({ after: "author", overlap: seconds(0.35), duration: seconds(1.5) }),
  render: stage({ after: "compile", overlap: seconds(0.4), duration: seconds(1.5) }),
  done: stage({ after: "render", overlap: seconds(0.55), duration: seconds(2.1) }),
});

const STEPS = ["PARSE TSX", "FREEZE IR", "LAYOUT", "DRAW", "ENCODE"];

export default function CodeBuild(ctx, props, signals) {
  const author = stageProgress(ctx.localFrame, ctx.fps, BUILD.author);
  const compile = stageProgress(ctx.localFrame, ctx.fps, BUILD.compile);
  const render = stageProgress(ctx.localFrame, ctx.fps, BUILD.render);
  const done = stageProgress(ctx.localFrame, ctx.fps, BUILD.done);
  return (
    <ThemeProvider value={releaseTheme}>
      <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: useTheme().colors.background }}>
        <Text key="eyebrow" className="absolute" style={{ left: 70, top: 48, fontSize: 16, letterSpacing: 4, color: useTheme().colors.accent }}>CODE → DETERMINISTIC FRAMES</Text>
        <Text key="title" className="absolute" style={{ left: 68, top: 84, width: 1100, fontSize: 54, color: useTheme().colors.ink, opacity: props.accentStrength }}>Author once. Seek any frame.</Text>
        <View key="editor" className="absolute overflow-hidden rounded-xl border border-slate-700 bg-slate-900 border-solid" style={{ left: 70, top: 190, width: 1040, height: 720, opacity: 0.25 + author * 0.75, filter: "drop-shadow(0px 30px 70px #00000099)" }}>
          <View key="toolbar" className="absolute flex items-center" style={{ borderStyle: "solid", left: 0, top: 0, width: 1040, height: 60, borderBottomWidth: 1, borderColor: useTheme().colors.hairline }}>
            <View key="red" style={{ marginLeft: 22, width: 12, height: 12, borderRadius: 7, backgroundColor: "#fb7185" }} />
            <View key="yellow" style={{ marginLeft: 10, width: 12, height: 12, borderRadius: 7, backgroundColor: "#fbbf24" }} />
            <View key="green" style={{ marginLeft: 10, width: 12, height: 12, borderRadius: 7, backgroundColor: "#34d399" }} />
            <Text key="file" style={{ marginLeft: 330, fontSize: 17, color: useTheme().colors.muted }}>release.motion.tsx</Text>
          </View>
          <Text key="code" className="absolute whitespace-pre-wrap tabular-nums line-clamp-12 overflow-hidden text-white" style={{ left: 34, top: 88, width: 970, height: 430, fontSize: 26, lineHeight: "38px", tabSize: 2, textOverflow: "ellipsis" }}>
            {"export default function Film(ctx) {\n"}
            <Span style={{ color: "#67e8f9" }}>{"  const p = ctx.hold.progress;\n"}</Span>
            {"  return (\n    <Scene className=\"h-full w-full\">\n"}
            <Span style={{ color: "#f9a8d4" }}>{"      <GeometryBatch progress={p} /> "}</Span>
            <Image key="status" src="asset://dot" style={{ width: 22, height: 22, verticalAlign: "middle", opacity: 0.35 + compile * 0.65 }} />
            {"\n      <Text>Every frame is pure.</Text>\n    </Scene>\n  );\n}"}
          </Text>
          <View key="terminal" className="absolute" style={{ borderStyle: "solid", left: 24, bottom: 24, width: 992, height: 142, borderRadius: 16, backgroundColor: "#020617", borderWidth: 1, borderColor: useTheme().colors.hairline }}>
            <Text key="command" className="absolute whitespace-pre tabular-nums" style={{ left: 22, top: 18, width: 940, height: 44, color: useTheme().colors.accent, fitText: fitText({ minFontSize: 16, maxFontSize: 28 }) }}>{"> valle motion studio release.motion.tsx --duration 3 --fps 60"}</Text>
            <Text key="result" className="absolute" style={{ left: 22, top: 82, fontSize: 24, color: "#86efac", opacity: done }}>{`✓ ${padNumber(round(done * 180), { width: 3 })} frames · byte stable`}</Text>
          </View>
        </View>
        <View key="pipeline" className="absolute" style={{ borderStyle: "solid", left: 1180, top: 190, width: 670, height: 720, borderRadius: useTheme().radius.panel, backgroundColor: useTheme().colors.panel, borderWidth: 1, borderColor: useTheme().colors.hairline }}>
          <Text key="pipeline-title" className="absolute" style={{ left: 34, top: 30, fontSize: 18, letterSpacing: 3, color: useTheme().colors.signal }}>BUILD PIPELINE</Text>
          {STEPS.map((step, index) => {
            const progress = interpolate(compile + render - index * 0.16, [0, 0.5], [0, 1], { easing: "easeOut" });
            return <View key={`step-${index}`} className="absolute" style={{ borderStyle: "solid", left: 34, top: 94 + index * 104, width: 602, height: 78, borderRadius: 16, backgroundColor: useTheme().colors.panelRaised, borderWidth: 1, borderColor: useTheme().colors.accent, opacity: 0.2 + progress * 0.8, translate: point((1 - progress) * 36, 0) }}><Text key={`index-${index}`} className="absolute" style={{ left: 20, top: 24, fontSize: 17, color: useTheme().colors.muted }}>{padNumber(index + 1, { width: 2 })}</Text><Text key={`step-label-${index}`} className="absolute" style={{ left: 86, top: 22, fontSize: 23, letterSpacing: 2, color: useTheme().colors.ink }}>{step}</Text><View key={`step-live-${index}`} className="absolute" style={{ right: 22, top: 29, width: 18, height: 18, borderRadius: 10, backgroundColor: "#86efac", opacity: progress, filter: "drop-shadow(0px 0px 10px #86efac)" }} /></View>;
          })}
        </View>
        <Text key="cue" className="absolute" style={{ right: 70, bottom: 46, fontSize: 15, letterSpacing: 2, color: useTheme().colors.signal, opacity: 0.5 + signals.narration.progress * 0.5 }}>CUE / AUTHOR → COMPILE → RENDER</Text>
      </Scene>
    </ThemeProvider>
  );
}
