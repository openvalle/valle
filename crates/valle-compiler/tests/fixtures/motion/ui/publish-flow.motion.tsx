export const component = "ui-publish-flow";

export const controls = defineControls({
  assets: { brandFont: asset({ kind: "font", required: true }) },
  props: {
    background: color({ default: "#07111f" }),
    surface: color({ default: "#111c2e" }),
    accent: color({ default: "#7c3aed" }),
    success: color({ default: "#10b981" }),
    text: color({ default: "#f8fafc" }),
  },
});

const FLOW = defineSequence({
  idle: stage({ duration: seconds(0.45) }),
  moveTitle: stage({ after: "idle", duration: seconds(0.75) }),
  focusTitle: stage({ after: "moveTitle", overlap: seconds(0.18), duration: seconds(0.55) }),
  typeTitle: stage({ after: "focusTitle", overlap: seconds(0.22), duration: seconds(1.05) }),
  movePublish: stage({ after: "typeTitle", overlap: seconds(0.18), duration: seconds(0.72) }),
  tap: stage({ after: "movePublish", overlap: seconds(0.08), duration: seconds(0.42) }),
  success: stage({ after: "tap", overlap: seconds(0.08), duration: seconds(1.25) }),
  settle: stage({ after: "success", overlap: seconds(0.42), duration: seconds(1.5) }),
});

const GRID = Array.from({ length: 1200 }, (_, index) => point(10 + (index % 60) * 14.7, 12 + Math.floor(index / 60) * 23.6));

function Cursor(ctx, { x, y, pressed, color }) {
  return (
    <View key="pointer" className="absolute" style={{ borderStyle: "solid", left: x, top: y, width: 18, height: 18, borderRadius: 10, backgroundColor: color, borderWidth: 3, borderColor: "#ffffff", transform: `scale(${1 - pressed * 0.24})`, filter: "drop-shadow(0px 6px 14px #02061788)" }}>
      <View key="tail" className="absolute" style={{ left: 13, top: 13, width: 13, height: 4, borderRadius: 2, backgroundColor: "#ffffff", transform: "rotate(45deg)" }} />
    </View>
  );
}

function Tap(ctx, { x, y, progress, color }) {
  return <View key="ring" className="absolute" style={{ borderStyle: "solid", left: x - 28, top: y - 28, width: 56, height: 56, borderRadius: 30, borderWidth: 3, borderColor: color, opacity: interpolate(progress, [0, 0.15, 1], [0, 1, 0]), transform: `scale(${0.35 + progress * 1.35})` }} />;
}

function Focus(ctx, { progress, color }) {
  return <View key="ring" className="absolute" style={{ borderStyle: "solid", left: 362, top: 332, width: 504, height: 66, borderRadius: 14, borderWidth: 2 + progress * 2, borderColor: color, opacity: progress, filter: `drop-shadow(0px 0px ${8 + progress * 10}px ${color})` }} />;
}

function StatePanel(ctx, { active, success, accent, text }) {
  const states = ["IDLE", "FOCUS", "TYPE", "PUBLISH", "DONE"];
  return (
    <View key="panel" className="absolute" style={{ borderStyle: "solid", left: 936, top: 116, width: 248, height: 444, borderRadius: 22, backgroundColor: "#08111fcc", borderWidth: 1, borderColor: "#ffffff1f" }}>
      <Text key="title" className="absolute" style={{ left: 20, top: 20, fontFamily: "asset://brandFont", fontSize: 13, letterSpacing: 2, color: "#94a3b8" }}>DECLARED STATE PLAN</Text>
      {states.map((state, index) => (
        <View key={`state-${state}`} className="absolute flex items-center" style={{ borderStyle: "solid", left: 18, top: 64 + index * 67, width: 212, height: 48, borderRadius: 13, backgroundColor: index === active ? (index === 4 ? success : accent) : "#132035", borderWidth: 1, borderColor: index <= active ? "#ffffff44" : "#ffffff12", opacity: 0.46 + (index <= active ? 0.54 : 0) }}>
          <View key={`dot-${index}`} style={{ marginLeft: 14, width: 10, height: 10, borderRadius: 6, backgroundColor: index <= active ? "#ffffff" : "#475569" }} />
          <Text key={`label-${index}`} style={{ marginLeft: 13, fontFamily: "asset://brandFont", fontSize: 15, letterSpacing: 1.4, color: text }}>{state}</Text>
          <Text key={`index-${index}`} style={{ marginLeft: "auto", marginRight: 14, fontFamily: "asset://brandFont", fontSize: 12, color: "#cbd5e1" }}>{padNumber(index + 1, { width: 2 })}</Text>
        </View>
      ))}
      <Text key="foot" className="absolute" style={{ left: 20, bottom: 18, fontFamily: "asset://brandFont", fontSize: 11, letterSpacing: 1.2, color: "#64748b" }}>NO EVENTS · NO MUTATION</Text>
    </View>
  );
}

function AppShell(ctx, { focus, typed, publish, success, surface, accent, successColor, text }) {
  return (
    <View key="window" className="absolute" style={{ borderStyle: "solid", left: 96, top: 116, width: 812, height: 500, borderRadius: 24, backgroundColor: surface, borderWidth: 1, borderColor: "#ffffff22", overflow: "hidden", filter: "drop-shadow(0px 28px 58px #02061799)" }}>
      <View key="chrome" className="absolute flex items-center" style={{ borderStyle: "solid", left: 0, top: 0, width: 812, height: 58, backgroundColor: "#0b1424", borderBottomWidth: 1, borderColor: "#ffffff12" }}>
        {["#fb7185", "#fbbf24", "#34d399"].map((color, index) => <View key={`dot-${index}`} style={{ marginLeft: index === 0 ? 20 : 9, width: 10, height: 10, borderRadius: 6, backgroundColor: color }} />)}
        <Text key="route" style={{ marginLeft: 22, fontFamily: "asset://brandFont", fontSize: 13, color: "#94a3b8" }}>studio.valle/publish</Text>
      </View>
      <View key="sidebar" className="absolute" style={{ borderStyle: "solid", left: 0, top: 58, width: 226, height: 442, backgroundColor: "#0d1728", borderRightWidth: 1, borderColor: "#ffffff12" }}>
        <Text key="brand" className="absolute" style={{ left: 22, top: 26, fontFamily: "asset://brandFont", fontSize: 24, color: text }}>VALLE</Text>
        {["PROJECT", "MOTION", "ASSETS", "EXPORT"].map((label, index) => <Text key={`nav-${index}`} className="absolute" style={{ left: 22, top: 88 + index * 52, fontFamily: "asset://brandFont", fontSize: 14, letterSpacing: 1.4, color: index === 1 ? "#c4b5fd" : "#64748b" }}>{label}</Text>)}
      </View>
      <View key="content" className="absolute" style={{ left: 226, top: 58, width: 586, height: 442, backgroundColor: "#111c2e" }}>
        <Text key="heading" className="absolute" style={{ left: 42, top: 34, fontFamily: "asset://brandFont", fontSize: 30, color: text }}>Publish motion</Text>
        <Text key="sub" className="absolute" style={{ left: 42, top: 76, fontFamily: "asset://brandFont", fontSize: 14, color: "#94a3b8" }}>Create a deterministic review link.</Text>
        <Text key="field-label" className="absolute" style={{ left: 42, top: 128, fontFamily: "asset://brandFont", fontSize: 13, letterSpacing: 1.2, color: "#94a3b8" }}>TITLE</Text>
        <View key="field" className="absolute" style={{ borderStyle: "solid", left: 40, top: 158, width: 504, height: 66, borderRadius: 14, backgroundColor: "#08111f", borderWidth: 1, borderColor: focus > 0 ? accent : "#334155" }}>
          <Text key="value" className="absolute" style={{ left: 18, top: 18, fontFamily: "asset://brandFont", fontSize: 20, color: text, opacity: 0.18 }}>Launch film / 4K master</Text>
          <View key="typed-clip" className="absolute" style={{ left: 18, top: 0, width: 292 * typed, height: 66, overflow: "hidden" }}>
            <Text key="typed" className="absolute" style={{ left: 0, top: 18, width: 292, fontFamily: "asset://brandFont", fontSize: 20, color: text }}>Launch film / 4K master</Text>
          </View>
          <View key="caret" className="absolute" style={{ left: 18 + 292 * typed, top: 17, width: 2, height: 30, backgroundColor: accent, opacity: focus }} />
        </View>
        <View key="option" className="absolute flex items-center" style={{ borderStyle: "solid", left: 40, top: 250, width: 504, height: 58, borderRadius: 13, backgroundColor: "#0d1728", borderWidth: 1, borderColor: "#ffffff12" }}>
          <View key="check" style={{ marginLeft: 16, width: 22, height: 22, borderRadius: 7, backgroundColor: success > 0 ? successColor : accent }} />
          <Text key="option-label" style={{ marginLeft: 13, fontFamily: "asset://brandFont", fontSize: 16, color: text }}>Include source map and controls</Text>
        </View>
        <View key="publish" className="absolute" style={{ right: 42, bottom: 42, width: 190, height: 58, borderRadius: 15, backgroundColor: success > 0 ? successColor : accent, opacity: 0.65 + publish * 0.35, transform: `scale(${1 - publish * 0.035})`, filter: `drop-shadow(0px 12px 24px ${success > 0 ? "#10b98155" : "#7c3aed55"})` }}>
          <Text key="publish-label" className="absolute" style={{ left: 0, top: 16, width: 190, fontFamily: "asset://brandFont", fontSize: 18, color: "#ffffff", textAlign: "center" }}>{success > 0.55 ? "Published" : "Publish"}</Text>
        </View>
      </View>
    </View>
  );
}

export default function PublishFlow(ctx, props) {
  const moveTitle = stageProgress(ctx.localFrame, ctx.fps, FLOW.moveTitle);
  const focus = stageProgress(ctx.localFrame, ctx.fps, FLOW.focusTitle);
  const typed = stageProgress(ctx.localFrame, ctx.fps, FLOW.typeTitle);
  const movePublish = stageProgress(ctx.localFrame, ctx.fps, FLOW.movePublish);
  const tap = stageProgress(ctx.localFrame, ctx.fps, FLOW.tap);
  const success = stageProgress(ctx.localFrame, ctx.fps, FLOW.success);
  const cursorX = interpolate(moveTitle, [0, 1], [1120, 706], { easing: "easeInOut" }) + interpolate(movePublish, [0, 1], [0, 52], { easing: "easeInOut" });
  const cursorY = interpolate(moveTitle, [0, 1], [620, 354], { easing: "easeInOut" }) + interpolate(movePublish, [0, 1], [0, 182], { easing: "easeInOut" });
  const active = success > 0.2 ? 4 : tap > 0.1 ? 3 : typed > 0.08 ? 2 : focus > 0.08 ? 1 : 0;

  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: props.background, fontFamily: "asset://brandFont" }} camera={{ center: point(640, 360), zoom: 1, rotation: 0 }}>
      <World key="world">
        <GeometryBatch key="grid" geometry="circle" positions={GRID} sizes={1.2} fills="#60a5fa" opacities={0.08} style={{ position: "absolute", left: 0, top: 0, width: 900, height: 500 }} />
        <AppShell key="app" focus={focus} typed={typed} publish={tap} success={success} surface={props.surface} accent={props.accent} successColor={props.success} text={props.text} />
        <Focus key="focus" progress={focus * (1 - movePublish)} color={props.accent} />
        <StatePanel key="states" active={active} success={props.success} accent={props.accent} text={props.text} />
        <Tap key="tap" x={cursorX + 9} y={cursorY + 9} progress={tap} color={props.success} />
        <Cursor key="cursor" x={cursorX} y={cursorY} pressed={tap} color={success > 0 ? props.success : props.accent} />
      </World>
      <Screen key="hud">
        <Text key="kicker" className="absolute" style={{ left: 96, top: 48, fontFamily: "asset://brandFont", fontSize: 16, letterSpacing: 3, color: "#a78bfa" }}>UI DEMO / DECLARED FLOW</Text>
        <Text key="folio" className="absolute" style={{ right: 96, top: 48, fontFamily: "asset://brandFont", fontSize: 15, letterSpacing: 1.5, color: "#94a3b8" }}>STATE ≠ EVENT LOOP</Text>
      </Screen>
    </Scene>
  );
}
