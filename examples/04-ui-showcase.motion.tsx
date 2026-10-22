// UI showcase: a neo-brutalist product demo. The settings card pops in, its rows slide up, a
// cursor flips the Focus toggle, the card expands into a detail view and Start opens a session.
// Shows components with props and children, ThemeProvider/useTheme, div/h1/p with Tailwind,
// `let`/`if` statements, defineLayoutStates/flip/layoutId, bounds() targets, visible and
// <Transition kind="circleOpen">.
// Render: valle motion render examples/04-ui-showcase.motion.tsx -o 04-ui-showcase.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

const THEME = {
  ink: "#111111",
  paper: "#FFFFFF",
  lime: "#A3E635",
  pink: "#FF4D8D",
  violet: "#7C3AED",
  yellow: "#FACC15",
  muted: "#3F3F46",
};
// Thick outlines and hard shadows without blur.
const OUTLINE = `5px solid ${THEME.ink}`;
const BOX = { border: OUTLINE, boxShadow: `8px 8px 0px ${THEME.ink}` };
const CHIP = { border: `4px solid ${THEME.ink}`, boxShadow: `4px 4px 0px ${THEME.ink}` };

const TIMES = {
  click: 2.0,
  expand: 2.55,
  press: 4.0,
  switchFrom: 4.1,
  switchTo: 4.65,
};

// A FLIP lays a box out at its final rectangle and scales it back from the first one, which
// would stretch a border. So the card is flat plates that each FLIP: a hard shadow, a black
// outline that also shows as the rule under the header, the header and the paper. Its content
// is laid out once per state.
const CARD = defineLayoutStates({
  compact: {
    shadow: rect(1117, 202, 630, 700),
    outline: rect(1105, 190, 630, 700),
    header: rect(1110, 195, 620, 120),
    paper: rect(1110, 320, 620, 565),
  },
  expanded: {
    shadow: rect(157, 117, 1630, 870),
    outline: rect(145, 105, 1630, 870),
    header: rect(150, 110, 1620, 140),
    paper: rect(150, 255, 1620, 715),
  },
});
const PLATES = [
  { id: "shadow", fill: THEME.ink },
  { id: "outline", fill: THEME.ink },
  { id: "header", fill: THEME.violet },
  { id: "paper", fill: THEME.paper },
];

const ROWS = [
  { id: "focus", title: "Focus mode", detail: "Mute every app", icon: THEME.pink, control: "toggle" },
  { id: "radio", title: "Lo-fi radio", detail: "Beats to think to", icon: THEME.violet, control: "ON" },
  { id: "breaks", title: "Break timer", detail: "Every 25 minutes", icon: THEME.yellow, control: "25m" },
  { id: "streak", title: "Streak", detail: "12 days in a row", icon: THEME.lime, control: "12" },
];
const OPTIONS = [
  { id: "notify", title: "Notifications", chip: "MUTED", fill: THEME.pink },
  { id: "music", title: "Lo-fi radio", chip: "ON", fill: THEME.lime },
  { id: "pause", title: "Break after", chip: "25 MIN", fill: THEME.yellow },
];
const START = { x: 1250, y: 800, width: 470, height: 112 };
const ARROW = path("M 0 0 L 0 52 L 13 40 L 23 61 L 33 56 L 23 36 L 41 36 Z");
const LIGHTNING = path("M 22 0 L 2 26 L 15 26 L 9 46 L 32 17 L 19 17 Z");

function Chip({ label, fill, tilt = 0 }) {
  const theme = useTheme();
  return (
    <div className="px-4 py-1" style={{ ...CHIP, backgroundColor: fill, rotate: `${tilt}deg` }}>
      <span style={{ fontSize: 24, fontWeight: 900, letterSpacing: 1, color: theme.ink }}>{label}</span>
    </div>
  );
}

function Toggle({ on }) {
  const theme = useTheme();
  return (
    <View key="track" className="relative"
      style={{ width: 104, height: 58, borderRadius: 29, border: OUTLINE,
        backgroundColor: interpolate(on, [0, 1], ["#E4E4E7", theme.pink]) }}>
      <View className="absolute" style={{ left: 5, top: 5, width: 38, height: 38, borderRadius: 19,
        border: OUTLINE, backgroundColor: theme.paper, translate: point(46 * on, 0) }} />
    </View>
  );
}

// A prepared condition picks one glyph; the other branches never reach the scene.
function Glyph({ kind }) {
  const { ink, paper } = useTheme();
  if (kind === "focus") {
    return (
      <View key="ring" className="flex items-center justify-center"
        style={{ width: 38, height: 38, borderRadius: 19, border: OUTLINE, backgroundColor: paper }}>
        <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: ink }} />
      </View>
    );
  }
  if (kind === "radio") {
    return (
      <div key="bars" className="flex items-end gap-1">
        <View style={{ width: 9, height: 18, backgroundColor: ink }} />
        <View style={{ width: 9, height: 34, backgroundColor: ink }} />
        <View style={{ width: 9, height: 26, backgroundColor: ink }} />
      </div>
    );
  }
  if (kind === "breaks") {
    return (
      <div key="pause" className="flex gap-2">
        <View style={{ width: 9, height: 30, backgroundColor: ink }} />
        <View style={{ width: 9, height: 30, backgroundColor: ink }} />
      </div>
    );
  }
  return <View key="bolt" style={{ width: 34, height: 46 }}><Path d={LIGHTNING} fill={ink} /></View>;
}

function Icon({ fill, kind }) {
  return (
    <div className="flex items-center justify-center"
      style={{ width: 72, height: 72, borderRadius: 16, border: OUTLINE, backgroundColor: fill }}>
      <Glyph kind={kind} />
    </div>
  );
}

function Row({ index, row, children }) {
  const { seconds } = useCurrentTime();
  const theme = useTheme();
  const rise = interpolate(seconds - index * 0.07, [0.45, 0.9], [0, 1], { easing: "easeOutCubic" });
  return (
    <div className="flex items-center gap-5 px-9"
      style={{ height: 140, borderBottom: index < ROWS.length - 1 ? `4px solid ${theme.ink}` : "none",
        opacity: rise, translate: point(0, 24 * (1 - rise)) }}>
      <Icon fill={row.icon} kind={row.id} />
      <div className="flex flex-col">
        <p style={{ fontSize: 32, fontWeight: 900, color: theme.ink }}>{row.title}</p>
        <p style={{ fontSize: 22, fontWeight: 600, color: theme.muted }}>{row.detail}</p>
      </div>
      <div className="flex-1" />
      {children}
    </div>
  );
}

function HomeScreen() {
  const { seconds } = useCurrentTime();
  const theme = useTheme();
  const pop = spring(seconds - 0.15, { duration: 0.6, bounce: 0.3 });
  const on = spring(seconds - TIMES.click - 0.03, { duration: 0.35, bounce: 0.3 });
  const expand = spring(seconds - TIMES.expand, { duration: 0.65, bounce: 0.15 });
  const compactShown = interpolate(seconds, [TIMES.expand, TIMES.expand + 0.15], [1, 0]);
  const detailShown = (index) => interpolate(seconds - index * 0.05, [TIMES.expand + 0.38, TIMES.expand + 0.68], [0, 1], { easing: "easeOutCubic" });
  const pressed = interpolate(seconds, [TIMES.press, TIMES.press + 0.08], [0, 1]);
  const hero = (index) => interpolate(seconds - index * 0.08, [0.05, 0.6], [0, 1], { easing: "easeOutCubic" });
  // The expanding card pushes the hero copy out.
  const heroOut = interpolate(seconds, [TIMES.expand + 0.05, TIMES.expand + 0.35], [0, 1], { easing: "easeInQuad" });
  return (
    <div className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080, backgroundColor: theme.lime }}>
      <div className="absolute flex flex-col items-start gap-6"
        style={{ left: 130, top: 290, width: 820, opacity: 1 - heroOut, translate: point(-60 * heroOut, 0) }}
        visible={seconds < TIMES.expand + 0.35}>
        <div style={{ opacity: hero(0), translate: point(-48 * (1 - hero(0)), 0) }}>
          <Chip label="NEW · FOCUS 2.0" fill={theme.yellow} tilt={-3} />
        </div>
        <h1 style={{ fontSize: 150, fontWeight: 900, lineHeight: 1, color: theme.ink,
          opacity: hero(1), translate: point(-48 * (1 - hero(1)), 0) }}>Focus,</h1>
        <div className="px-5" style={{ ...BOX, backgroundColor: theme.pink, rotate: "-2deg",
          opacity: hero(2), translate: point(-48 * (1 - hero(2)), 0) }}>
          <h1 style={{ fontSize: 150, fontWeight: 900, lineHeight: 1.1, color: theme.ink }}>finally.</h1>
        </div>
        <p style={{ width: 640, fontSize: 34, fontWeight: 600, lineHeight: 1.35, color: theme.ink,
          opacity: hero(3), translate: point(-48 * (1 - hero(3)), 0) }}>
          One tap mutes the noise.
        </p>
      </div>

      <div className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080, opacity: clamp(pop * 2, 0, 1),
        scale: 0.9 + 0.1 * pop, translate: point(0, 40 * (1 - pop)), transformOrigin: "1420px 540px" }}>
        {PLATES.map((plate) => (
          <View key={plate.id} layoutId={plate.id}
            style={{ layoutTransition: flip(CARD, "compact", "expanded", expand), backgroundColor: plate.fill }} />
        ))}

        <div className="absolute" style={{ left: 1110, top: 195, width: 620, height: 690, opacity: compactShown }}
          visible={seconds < TIMES.expand + 0.15}>
          <div className="flex items-center justify-between px-9" style={{ height: 120 }}>
            <h1 style={{ fontSize: 48, fontWeight: 900, color: theme.paper }}>Settings</h1>
            <View style={{ width: 60, height: 60, borderRadius: 30, border: OUTLINE, backgroundColor: theme.yellow }} />
          </div>
          <div className="flex flex-col" style={{ marginTop: 5 }}>
            {ROWS.map((row, index) => (
              <Row key={row.id} index={index} row={row}>
                {row.control === "toggle" ? <Toggle key="toggle" on={on} /> : <Chip key="chip" label={row.control} fill={row.icon} />}
              </Row>
            ))}
          </div>
        </div>

        <div className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080 }} visible={seconds >= TIMES.expand + 0.3}>
          <div className="absolute flex items-center justify-between px-12"
            style={{ left: 150, top: 110, width: 1620, height: 140, opacity: detailShown(0) }}>
            <h1 style={{ fontSize: 72, fontWeight: 900, color: theme.paper }}>Focus mode</h1>
            <Chip label="ON" fill={theme.lime} tilt={2} />
          </div>
          <div className="absolute flex flex-col items-center justify-center"
            style={{ ...BOX, left: 210, top: 315, width: 680, height: 590, backgroundColor: theme.yellow,
              opacity: detailShown(1), translate: point(0, 24 * (1 - detailShown(1))) }}>
            <Text style={{ fontSize: 230, fontWeight: 900, lineHeight: 1, color: theme.ink }}>25:00</Text>
            <Text style={{ fontSize: 28, fontWeight: 900, letterSpacing: 3, color: theme.ink }}>MINUTES OF DEEP WORK</Text>
          </div>
          {OPTIONS.map((option, index) => (
            <div key={option.id} className="absolute flex items-center justify-between px-8"
              style={{ ...BOX, left: 960, top: 315 + index * 150, width: 760, height: 120, backgroundColor: theme.paper,
                opacity: detailShown(index + 2), translate: point(0, 24 * (1 - detailShown(index + 2))) }}>
              <p style={{ fontSize: 34, fontWeight: 900, color: theme.ink }}>{option.title}</p>
              <Chip label={option.chip} fill={option.fill} />
            </div>
          ))}
          <div key="start" className="absolute flex items-center justify-center"
            style={{ ...BOX, left: START.x, top: START.y, width: START.width, height: START.height, backgroundColor: theme.pink,
              boxShadow: `${8 - 8 * pressed}px ${8 - 8 * pressed}px 0px ${theme.ink}`,
              opacity: detailShown(5), translate: point(8 * pressed, 24 * (1 - detailShown(5)) + 8 * pressed) }}>
            <Text style={{ fontSize: 42, fontWeight: 900, color: theme.ink }}>Start session</Text>
          </div>
        </div>
      </div>
    </div>
  );
}

function FocusScreen() {
  const { seconds } = useCurrentTime();
  const { duration } = useComposition();
  const theme = useTheme();
  // Shapes pop in while the circle opens; text waits until the switch is over.
  const shape = (index) => spring(seconds - TIMES.switchFrom - 0.2 - index * 0.08, { duration: 0.55, bounce: 0.3 });
  const enter = (index) => spring(seconds - TIMES.switchTo - index * 0.07, { duration: 0.55, bounce: 0.3 });
  const elapsed = interpolate(seconds, [TIMES.switchTo, duration], [0.02, 0.14], { easing: "easeOutSine" });
  return (
    <div className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080, backgroundColor: theme.violet }}>
      <View className="absolute" style={{ left: 1420, top: -170, width: 460, height: 460, borderRadius: 230,
        border: OUTLINE, backgroundColor: theme.yellow, scale: 0.6 + 0.4 * shape(0) }} />
      <View className="absolute" style={{ left: -50, top: 860, width: 220, height: 220, border: OUTLINE,
        backgroundColor: theme.lime, rotate: `${18 + 12 * seconds}deg`, scale: shape(1) }} />
      <View className="absolute" style={{ left: 980, top: 930, width: 90, height: 90, borderRadius: 45,
        border: OUTLINE, backgroundColor: theme.pink, scale: shape(2) }} />

      <div className="absolute flex flex-col items-start gap-7" style={{ left: 140, top: 340, width: 960 }}>
        <div style={{ opacity: clamp(enter(0) * 2, 0, 1), scale: 0.5 + 0.5 * enter(0) }}>
          <Chip label="SESSION 1 OF 4" fill={theme.paper} tilt={-3} />
        </div>
        <h1 style={{ fontSize: 156, fontWeight: 900, lineHeight: 1, color: theme.paper, textShadow: `8px 8px 0px ${theme.ink}`,
          opacity: clamp(enter(1) * 2, 0, 1), translate: point(0, 40 * (1 - enter(1))) }}>Deep work</h1>
        <p style={{ width: 920, fontSize: 36, fontWeight: 700, lineHeight: 1.35, color: "#F5F3FF",
          opacity: clamp(enter(2) * 2, 0, 1), translate: point(0, 24 * (1 - enter(2))) }}>
          Notifications are off. Just you and the work.
        </p>
        <div className="flex gap-5" style={{ marginTop: 16 }}>
          {OPTIONS.map((option, index) => (
            <div key={`${option.id}-tag`} style={{ opacity: clamp(enter(index + 3) * 2, 0, 1), scale: 0.5 + 0.5 * enter(index + 3) }}>
              <Chip label={option.chip} fill={option.fill} tilt={index === 1 ? 2 : -2} />
            </div>
          ))}
        </div>
      </div>

      <div className="absolute flex flex-col items-center justify-center"
        style={{ ...BOX, left: 1190, top: 270, width: 560, height: 560, borderRadius: 40, backgroundColor: theme.paper,
          scale: 0.7 + 0.3 * enter(1), opacity: clamp(enter(1) * 2, 0, 1) }}>
        <Path d={arc(point(275, 275), 205, deg(-90), deg(270))} fill="none" stroke="#EDE9FE" strokeWidth={40}
          style={{ position: "absolute", left: 0, top: 0 }} />
        <Path d={arc(point(275, 275), 205, deg(-90), deg(270))} fill="none" stroke={theme.pink} strokeWidth={40}
          strokeLinecap="round" trimEnd={elapsed} style={{ position: "absolute", left: 0, top: 0 }} />
        <Text style={{ fontSize: 120, fontWeight: 900, lineHeight: 1, color: theme.ink }}>24:58</Text>
        <Text style={{ fontSize: 26, fontWeight: 800, letterSpacing: 3, color: theme.muted }}>REMAINING</Text>
      </div>
    </div>
  );
}

// After the switch only the session screen is left. Before it, the home screen paints alone;
// during it, a circle transition from nothing to the session screen opens over the home screen.
function Screens() {
  const { seconds } = useCurrentTime();
  if (seconds >= TIMES.switchTo) return <FocusScreen key="focus" />;
  const progress = interpolate(seconds, [TIMES.switchFrom, TIMES.switchTo], [0, 1], { easing: "easeInOutCubic" });
  return (
    <>
      <HomeScreen key="home" />
      <Transition key="switch" kind="circleOpen" progress={progress} visible={seconds >= TIMES.switchFrom}
        params={{ centerX: (START.x + START.width / 2) / 1920, centerY: (START.y + START.height / 2) / 1080, softness: 2 }}
        style={{ position: "absolute", left: 0, top: 0, width: 1920, height: 1080 }}>
        <View key="nothing" />
        <FocusScreen key="focus-in" />
      </Transition>
    </>
  );
}

function Cursor() {
  const { seconds } = useCurrentTime();
  const theme = useTheme();
  const toggle = bounds("screens/home/focus/toggle/track");
  const button = bounds("screens/home/start");
  const park = point(1540, 1010);
  const target = point(toggle.x + toggle.width * 0.7, toggle.y + toggle.height * 0.6);
  const aside = point(target.x + 70, target.y + 120);
  const press = point(button.x + button.width * 0.62, button.y + button.height * 0.62);
  let x = park.x;
  let y = park.y;
  if (seconds >= 2.25) {
    const u = interpolate(seconds, [3.25, 3.95], [0, 1], { easing: "easeInOutSine" });
    const v = interpolate(seconds, [2.25, 2.6], [0, 1], { easing: "easeOutCubic" });
    x = target.x + (aside.x - target.x) * v + (press.x - aside.x) * u;
    y = target.y + (aside.y - target.y) * v + (press.y - aside.y) * u;
  } else {
    const u = interpolate(seconds, [0.85, 1.95], [0, 1], { easing: "easeInOutSine" });
    x = park.x + (target.x - park.x) * u;
    y = park.y + (target.y - park.y) * u + 60 * sin(u * TAU / 2);
  }
  const tap = interpolate(seconds, [TIMES.click, TIMES.click + 0.07, TIMES.click + 0.2], [1, 0.82, 1])
    * interpolate(seconds, [TIMES.press, TIMES.press + 0.07, TIMES.press + 0.2], [1, 0.82, 1]);
  const shown = interpolate(seconds, [0.6, 0.85, TIMES.switchFrom, TIMES.switchFrom + 0.3], [0, 1, 1, 0]);
  const ripple = (at) => interpolate(seconds, [at, at + 0.45], [0, 1], { easing: "easeOutCubic" });
  return (
    <div className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080 }}>
      <Circle cx={target.x} cy={target.y} r={14 + 46 * ripple(TIMES.click)} fill="none" stroke={theme.ink} strokeWidth={5}
        style={{ opacity: 1 - ripple(TIMES.click) }} visible={seconds >= TIMES.click && seconds < TIMES.click + 0.45} />
      <Circle cx={press.x} cy={press.y} r={14 + 46 * ripple(TIMES.press)} fill="none" stroke={theme.ink} strokeWidth={5}
        style={{ opacity: 1 - ripple(TIMES.press) }} visible={seconds >= TIMES.press && seconds < TIMES.press + 0.45} />
      <View className="absolute" style={{ left: 0, top: 0, width: 50, height: 70, opacity: shown,
        translate: point(x, y), scale: tap, transformOrigin: "0px 0px" }}>
        <Path d={ARROW} fill={theme.ink} style={{ translate: point(5, 5) }} />
        <Path d={ARROW} fill={theme.paper} stroke={theme.ink} strokeWidth={4} strokeLinejoin="round" />
      </View>
    </div>
  );
}

export default function UiShowcase() {
  return (
    <ThemeProvider value={THEME}>
      <Scene className="relative h-full w-full" style={{ backgroundColor: useTheme().lime }}>
        <Screens key="screens" />
        <Cursor key="cursor" />
      </Scene>
    </ThemeProvider>
  );
}
