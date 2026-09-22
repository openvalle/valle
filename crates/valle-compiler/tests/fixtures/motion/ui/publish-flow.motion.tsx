export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

export const controls = {
  assets: { brandFont: asset({ kind: "font", required: true }) },
  props: {
    background: color({ default: "#07111f" }),
    surface: color({ default: "#111c2e" }),
    accent: color({ default: "#7c3aed" }),
    success: color({ default: "#10b981" }),
    text: color({ default: "#f8fafc" }),
  },
};

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

const GRID = Array.from({ length: 1200 }, (_, index) => point(15 + (index % 60) * 22.05, 18 + Math.floor(index / 60) * 35.4));

function Cursor(ctx, { x, y, pressed, color }) {
  return (
    <View key="pointer" className="absolute" style={{ borderStyle: "solid", left: x, top: y, width: 27, height: 27, borderRadius: 15, backgroundColor: color, borderWidth: 4.5, borderColor: "#ffffff", transform: `scale(${1 - pressed * 0.24})`, filter: "drop-shadow(0px 6px 14px #02061788)" }}>
      <View key="tail" className="absolute" style={{ left: 19.5, top: 19.5, width: 19.5, height: 6, borderRadius: 3, backgroundColor: "#ffffff", transform: "rotate(45deg)" }} />
    </View>
  );
}

function Tap(ctx, { x, y, progress, color }) {
  return <View key="ring" className="absolute" style={{ borderStyle: "solid", left: x - 42, top: y - 42, width: 84, height: 84, borderRadius: 45, borderWidth: 4.5, borderColor: color, opacity: interpolate(progress, [0, 0.15, 1], [0, 1, 0]), transform: `scale(${0.35 + progress * 1.35})` }} />;
}

function Focus(ctx, { progress, color }) {
  return <View key="ring" className="absolute" style={{ borderStyle: "solid", left: 543, top: 498, width: 756, height: 99, borderRadius: 21, borderWidth: 3 + progress * 3, borderColor: color, opacity: progress, filter: `drop-shadow(0px 0px ${12 + progress * 15}px ${color})` }} />;
}

function StatePanel(ctx, { active, success, accent, text }) {
  const states = ["IDLE", "FOCUS", "TYPE", "PUBLISH", "DONE"];
  return (
    <View key="panel" className="absolute" style={{ borderStyle: "solid", left: 1404, top: 174, width: 372, height: 666, borderRadius: 33, backgroundColor: "#08111fcc", borderWidth: 1.5, borderColor: "#ffffff1f" }}>
      <Text key="title" className="absolute" style={{ left: 30, top: 30, fontFamily: "asset://brandFont", fontSize: 19.5, letterSpacing: 3, color: "#94a3b8" }}>DECLARED STATE PLAN</Text>
      {states.map((state, index) => (
        <View key={`state-${state}`} className="absolute flex items-center" style={{ borderStyle: "solid", left: 27, top: 96 + index * 100.5, width: 318, height: 72, borderRadius: 19.5, backgroundColor: index === active ? (index === 4 ? success : accent) : "#132035", borderWidth: 1.5, borderColor: index <= active ? "#ffffff44" : "#ffffff12", opacity: 0.46 + (index <= active ? 0.54 : 0) }}>
          <View key={`dot-${index}`} style={{ marginLeft: 21, width: 15, height: 15, borderRadius: 9, backgroundColor: index <= active ? "#ffffff" : "#475569" }} />
          <Text key={`label-${index}`} style={{ marginLeft: 19.5, fontFamily: "asset://brandFont", fontSize: 22.5, letterSpacing: 2.1, color: text }}>{state}</Text>
          <Text key={`index-${index}`} style={{ marginLeft: "auto", marginRight: 21, fontFamily: "asset://brandFont", fontSize: 18, color: "#cbd5e1" }}>{padNumber(index + 1, { width: 2 })}</Text>
        </View>
      ))}
      <Text key="foot" className="absolute" style={{ left: 30, bottom: 27, fontFamily: "asset://brandFont", fontSize: 16.5, letterSpacing: 1.8, color: "#64748b" }}>NO EVENTS · NO MUTATION</Text>
    </View>
  );
}

function AppShell(ctx, { focus, typed, publish, success, surface, accent, successColor, text }) {
  return (
    <View key="window" className="absolute" style={{ borderStyle: "solid", left: 144, top: 174, width: 1218, height: 750, borderRadius: 36, backgroundColor: surface, borderWidth: 1.5, borderColor: "#ffffff22", overflow: "hidden", filter: "drop-shadow(0px 28px 58px #02061799)" }}>
      <View key="chrome" className="absolute flex items-center" style={{ borderStyle: "solid", left: 0, top: 0, width: 1218, height: 87, backgroundColor: "#0b1424", borderBottomWidth: 1.5, borderColor: "#ffffff12" }}>
        {["#fb7185", "#fbbf24", "#34d399"].map((color, index) => <View key={`dot-${index}`} style={{ marginLeft: index === 0 ? 30 : 13.5, width: 15, height: 15, borderRadius: 9, backgroundColor: color }} />)}
        <Text key="route" style={{ marginLeft: 33, fontFamily: "asset://brandFont", fontSize: 19.5, color: "#94a3b8" }}>studio.valle/publish</Text>
      </View>
      <View key="sidebar" className="absolute" style={{ borderStyle: "solid", left: 0, top: 87, width: 339, height: 663, backgroundColor: "#0d1728", borderRightWidth: 1.5, borderColor: "#ffffff12" }}>
        <Text key="brand" className="absolute" style={{ left: 33, top: 39, fontFamily: "asset://brandFont", fontSize: 36, color: text }}>VALLE</Text>
        {["PROJECT", "MOTION", "ASSETS", "EXPORT"].map((label, index) => <Text key={`nav-${index}`} className="absolute" style={{ left: 33, top: 132 + index * 78, fontFamily: "asset://brandFont", fontSize: 21, letterSpacing: 2.1, color: index === 1 ? "#c4b5fd" : "#64748b" }}>{label}</Text>)}
      </View>
      <View key="content" className="absolute" style={{ left: 339, top: 87, width: 879, height: 663, backgroundColor: "#111c2e" }}>
        <Text key="heading" className="absolute" style={{ left: 63, top: 51, fontFamily: "asset://brandFont", fontSize: 45, color: text }}>Publish motion</Text>
        <Text key="sub" className="absolute" style={{ left: 63, top: 114, fontFamily: "asset://brandFont", fontSize: 21, color: "#94a3b8" }}>Create a deterministic review link.</Text>
        <Text key="field-label" className="absolute" style={{ left: 63, top: 192, fontFamily: "asset://brandFont", fontSize: 19.5, letterSpacing: 1.8, color: "#94a3b8" }}>TITLE</Text>
        <View key="field" className="absolute" style={{ borderStyle: "solid", left: 60, top: 237, width: 756, height: 99, borderRadius: 21, backgroundColor: "#08111f", borderWidth: 1.5, borderColor: focus > 0 ? accent : "#334155" }}>
          <Text key="value" className="absolute" style={{ left: 27, top: 27, fontFamily: "asset://brandFont", fontSize: 30, color: text, opacity: 0.18 }}>Launch film / 4K master</Text>
          <View key="typed-clip" className="absolute" style={{ left: 27, top: 0, width: 438 * typed, height: 99, overflow: "hidden" }}>
            <Text key="typed" className="absolute" style={{ left: 0, top: 27, width: 438, fontFamily: "asset://brandFont", fontSize: 30, color: text }}>Launch film / 4K master</Text>
          </View>
          <View key="caret" className="absolute" style={{ left: 27 + 438 * typed, top: 25.5, width: 3, height: 45, backgroundColor: accent, opacity: focus }} />
        </View>
        <View key="option" className="absolute flex items-center" style={{ borderStyle: "solid", left: 60, top: 375, width: 756, height: 87, borderRadius: 19.5, backgroundColor: "#0d1728", borderWidth: 1.5, borderColor: "#ffffff12" }}>
          <View key="check" style={{ marginLeft: 24, width: 33, height: 33, borderRadius: 10.5, backgroundColor: success > 0 ? successColor : accent }} />
          <Text key="option-label" style={{ marginLeft: 19.5, fontFamily: "asset://brandFont", fontSize: 24, color: text }}>Include source map and controls</Text>
        </View>
        <View key="publish" className="absolute" style={{ right: 63, bottom: 63, width: 285, height: 87, borderRadius: 22.5, backgroundColor: success > 0 ? successColor : accent, opacity: 0.65 + publish * 0.35, transform: `scale(${1 - publish * 0.035})`, filter: `drop-shadow(0px 12px 24px ${success > 0 ? "#10b98155" : "#7c3aed55"})` }}>
          <Text key="publish-label" className="absolute" style={{ left: 0, top: 24, width: 285, fontFamily: "asset://brandFont", fontSize: 27, color: "#ffffff", textAlign: "center" }}>{success > 0.55 ? "Published" : "Publish"}</Text>
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
  const cursorX = interpolate(moveTitle, [0, 1], [1680, 1059], { easing: "easeInOut" }) + interpolate(movePublish, [0, 1], [0, 78], { easing: "easeInOut" });
  const cursorY = interpolate(moveTitle, [0, 1], [930, 531], { easing: "easeInOut" }) + interpolate(movePublish, [0, 1], [0, 273], { easing: "easeInOut" });
  const active = success > 0.2 ? 4 : tap > 0.1 ? 3 : typed > 0.08 ? 2 : focus > 0.08 ? 1 : 0;

  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: props.background, fontFamily: "asset://brandFont" }} camera={{ center: point(960, 540), zoom: 1, rotation: 0 }}>
      <World key="world">
        <GeometryBatch key="grid" geometry="circle" positions={GRID} sizes={1.8} fills="#60a5fa" opacities={0.08} style={{ position: "absolute", left: 0, top: 0, width: 1350, height: 750 }} />
        <AppShell key="app" focus={focus} typed={typed} publish={tap} success={success} surface={props.surface} accent={props.accent} successColor={props.success} text={props.text} />
        <Focus key="focus" progress={focus * (1 - movePublish)} color={props.accent} />
        <StatePanel key="states" active={active} success={props.success} accent={props.accent} text={props.text} />
        <Tap key="tap" x={cursorX + 13.5} y={cursorY + 13.5} progress={tap} color={props.success} />
        <Cursor key="cursor" x={cursorX} y={cursorY} pressed={tap} color={success > 0 ? props.success : props.accent} />
      </World>
      <Screen key="hud">
        <Text key="kicker" className="absolute" style={{ left: 144, top: 72, fontFamily: "asset://brandFont", fontSize: 24, letterSpacing: 4.5, color: "#a78bfa" }}>UI DEMO / DECLARED FLOW</Text>
        <Text key="folio" className="absolute" style={{ right: 144, top: 72, fontFamily: "asset://brandFont", fontSize: 22.5, letterSpacing: 2.25, color: "#94a3b8" }}>STATE ≠ EVENT LOOP</Text>
      </Screen>
    </Scene>
  );
}
