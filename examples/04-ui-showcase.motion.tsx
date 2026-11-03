// UI showcase: a neo-brutalist product demo. The settings card pops in, its rows slide up, a
// cursor flips the Focus toggle, the card expands into a detail view and Start opens a session.
// Shows components with props and children, a theme as module constants, View/Text with
// Tailwind, `let`/`if` statements, defineLayoutStates/flip/layoutId, bounds() targets,
// conditional rendering and <Transition kind="circleOpen">.
// Render: valle motion render examples/04-ui-showcase.motion.tsx -o 04-ui-showcase.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

// Easings.
const IN_OUT_CUBIC = "cubic-bezier(0.65, 0, 0.35, 1)";
const IN_OUT_SINE = "cubic-bezier(0.37, 0, 0.63, 1)";
const IN_QUAD = "cubic-bezier(0.11, 0, 0.5, 0)";
const OUT_CUBIC = "cubic-bezier(0.33, 1, 0.68, 1)";
const OUT_SINE = "cubic-bezier(0.61, 1, 0.88, 1)";

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
    shadow: { x: 1117, y: 202, width: 630, height: 700 },
    outline: { x: 1105, y: 190, width: 630, height: 700 },
    header: { x: 1110, y: 195, width: 620, height: 120 },
    paper: { x: 1110, y: 320, width: 620, height: 565 },
  },
  expanded: {
    shadow: { x: 157, y: 117, width: 1630, height: 870 },
    outline: { x: 145, y: 105, width: 1630, height: 870 },
    header: { x: 150, y: 110, width: 1620, height: 140 },
    paper: { x: 150, y: 255, width: 1620, height: 715 },
  },
});
const PLATES = [
  { id: "shadow", fill: THEME.ink },
  { id: "outline", fill: THEME.ink },
  { id: "header", fill: THEME.violet },
  { id: "paper", fill: THEME.paper },
];

const ROWS = [
  {
    id: "focus",
    title: "Focus mode",
    detail: "Mute every app",
    icon: THEME.pink,
    control: "toggle",
  },
  {
    id: "radio",
    title: "Lo-fi radio",
    detail: "Beats to think to",
    icon: THEME.violet,
    control: "ON",
  },
  {
    id: "breaks",
    title: "Break timer",
    detail: "Every 25 minutes",
    icon: THEME.yellow,
    control: "25m",
  },
  { id: "streak", title: "Streak", detail: "12 days in a row", icon: THEME.lime, control: "12" },
];
const OPTIONS = [
  { id: "notify", title: "Notifications", chip: "MUTED", fill: THEME.pink },
  { id: "music", title: "Lo-fi radio", chip: "ON", fill: THEME.lime },
  { id: "pause", title: "Break after", chip: "25 MIN", fill: THEME.yellow },
];
// The session ring, from 12 o'clock round clockwise.
const RING = "M 275 70 A 205 205 0 1 1 275 480 A 205 205 0 1 1 275 70";
const START = { x: 1250, y: 800, width: 470, height: 112 };
const ARROW = "M 0 0 L 0 52 L 13 40 L 23 61 L 33 56 L 23 36 L 41 36 Z";
const LIGHTNING = "M 22 0 L 2 26 L 15 26 L 9 46 L 32 17 L 19 17 Z";

function Chip({ label, fill, tilt = 0 }) {
  return (
    <View className="px-4 py-1" style={{ ...CHIP, backgroundColor: fill, rotate: `${tilt}deg` }}>
      <Text style={{ fontSize: 24, fontWeight: 900, letterSpacing: 1, color: THEME.ink }}>
        {label}
      </Text>
    </View>
  );
}

function Toggle({ on }) {
  return (
    <View
      key="track"
      className="relative"
      style={{
        width: 104,
        height: 58,
        borderRadius: 29,
        border: OUTLINE,
        backgroundColor: interpolate(on, [0, 1], ["#E4E4E7", THEME.pink]),
      }}
    >
      <View
        className="absolute"
        style={{
          left: 5,
          top: 5,
          width: 38,
          height: 38,
          borderRadius: 19,
          border: OUTLINE,
          backgroundColor: THEME.paper,
          translate: `${46 * on}px 0px`,
        }}
      />
    </View>
  );
}

// A prepared condition picks one glyph; the other branches never reach the scene.
function Glyph({ kind }) {
  const { ink, paper } = THEME;
  if (kind === "focus") {
    return (
      <View
        key="ring"
        className="flex items-center justify-center"
        style={{ width: 38, height: 38, borderRadius: 19, border: OUTLINE, backgroundColor: paper }}
      >
        <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: ink }} />
      </View>
    );
  }
  if (kind === "radio") {
    return (
      <View key="bars" className="flex items-end gap-1">
        <View style={{ width: 9, height: 18, backgroundColor: ink }} />
        <View style={{ width: 9, height: 34, backgroundColor: ink }} />
        <View style={{ width: 9, height: 26, backgroundColor: ink }} />
      </View>
    );
  }
  if (kind === "breaks") {
    return (
      <View key="pause" className="flex gap-2">
        <View style={{ width: 9, height: 30, backgroundColor: ink }} />
        <View style={{ width: 9, height: 30, backgroundColor: ink }} />
      </View>
    );
  }
  return (
    <svg key="bolt" width={34} height={46}>
      <path d={LIGHTNING} fill={ink} />
    </svg>
  );
}

function Icon({ fill, kind }) {
  return (
    <View
      className="flex items-center justify-center"
      style={{ width: 72, height: 72, borderRadius: 16, border: OUTLINE, backgroundColor: fill }}
    >
      <Glyph kind={kind} />
    </View>
  );
}

function Row({ index, row, children }) {
  const { seconds } = useCurrentTime();
  const rise = interpolate(seconds - index * 0.07, [0.45, 0.9], [0, 1], { easing: OUT_CUBIC });
  return (
    <View
      className="flex items-center gap-5 px-9"
      style={{
        height: 140,
        borderBottom: index < ROWS.length - 1 ? `4px solid ${THEME.ink}` : "none",
        opacity: rise,
        translate: `0px ${24 * (1 - rise)}px`,
      }}
    >
      <Icon fill={row.icon} kind={row.id} />
      <View className="flex flex-col">
        <Text style={{ fontSize: 32, fontWeight: 900, color: THEME.ink }}>{row.title}</Text>
        <Text style={{ fontSize: 22, fontWeight: 600, color: THEME.muted }}>{row.detail}</Text>
      </View>
      <View className="flex-1" />
      {children}
    </View>
  );
}

function HomeScreen() {
  const { seconds } = useCurrentTime();
  const pop = spring(seconds - 0.15, { duration: 0.6, bounce: 0.3 });
  const on = spring(seconds - TIMES.click - 0.03, { duration: 0.35, bounce: 0.3 });
  const expand = spring(seconds - TIMES.expand, { duration: 0.65, bounce: 0.15 });
  const compactShown = interpolate(seconds, [TIMES.expand, TIMES.expand + 0.15], [1, 0]);
  const detailShown = (index) =>
    interpolate(seconds - index * 0.05, [TIMES.expand + 0.38, TIMES.expand + 0.68], [0, 1], {
      easing: OUT_CUBIC,
    });
  const pressed = interpolate(seconds, [TIMES.press, TIMES.press + 0.08], [0, 1]);
  const hero = (index) =>
    interpolate(seconds - index * 0.08, [0.05, 0.6], [0, 1], { easing: OUT_CUBIC });
  // The expanding card pushes the hero copy out.
  const heroOut = interpolate(seconds, [TIMES.expand + 0.05, TIMES.expand + 0.35], [0, 1], {
    easing: IN_QUAD,
  });
  return (
    <View
      className="absolute"
      style={{ left: 0, top: 0, width: 1920, height: 1080, backgroundColor: THEME.lime }}
    >
      <View
        className="absolute flex flex-col items-start gap-6"
        style={{
          left: 130,
          top: 290,
          width: 820,
          opacity: 1 - heroOut,
          translate: `${-60 * heroOut}px 0px`,
          visibility: seconds < TIMES.expand + 0.35 ? "visible" : "hidden",
        }}
      >
        <View style={{ opacity: hero(0), translate: `${-48 * (1 - hero(0))}px 0px` }}>
          <Chip label="NEW · FOCUS 2.0" fill={THEME.yellow} tilt={-3} />
        </View>
        <Text
          style={{
            fontSize: 150,
            fontWeight: 900,
            lineHeight: 1,
            color: THEME.ink,
            opacity: hero(1),
            translate: `${-48 * (1 - hero(1))}px 0px`,
          }}
        >
          Focus,
        </Text>
        <View
          className="px-5"
          style={{
            ...BOX,
            backgroundColor: THEME.pink,
            rotate: "-2deg",
            opacity: hero(2),
            translate: `${-48 * (1 - hero(2))}px 0px`,
          }}
        >
          <Text style={{ fontSize: 150, fontWeight: 900, lineHeight: 1.1, color: THEME.ink }}>
            finally.
          </Text>
        </View>
        <Text
          style={{
            width: 640,
            fontSize: 34,
            fontWeight: 600,
            lineHeight: 1.35,
            color: THEME.ink,
            opacity: hero(3),
            translate: `${-48 * (1 - hero(3))}px 0px`,
          }}
        >
          One tap mutes the noise.
        </Text>
      </View>

      <View
        className="absolute"
        style={{
          left: 0,
          top: 0,
          width: 1920,
          height: 1080,
          opacity: clamp(pop * 2, 0, 1),
          scale: 0.9 + 0.1 * pop,
          translate: `0px ${40 * (1 - pop)}px`,
          transformOrigin: "1420px 540px",
        }}
      >
        {PLATES.map((plate) => (
          <View
            key={plate.id}
            layoutId={plate.id}
            style={{
              layoutTransition: flip(CARD, "compact", "expanded", expand),
              backgroundColor: plate.fill,
            }}
          />
        ))}

        <View
          className="absolute"
          style={{
            left: 1110,
            top: 195,
            width: 620,
            height: 690,
            opacity: compactShown,
            visibility: seconds < TIMES.expand + 0.15 ? "visible" : "hidden",
          }}
        >
          <View className="flex items-center justify-between px-9" style={{ height: 120 }}>
            <Text style={{ fontSize: 48, fontWeight: 900, color: THEME.paper }}>Settings</Text>
            <View
              style={{
                width: 60,
                height: 60,
                borderRadius: 30,
                border: OUTLINE,
                backgroundColor: THEME.yellow,
              }}
            />
          </View>
          <View className="flex flex-col" style={{ marginTop: 5 }}>
            {ROWS.map((row, index) => (
              <Row key={row.id} index={index} row={row}>
                {row.control === "toggle" ? (
                  <Toggle key="toggle" on={on} />
                ) : (
                  <Chip key="chip" label={row.control} fill={row.icon} />
                )}
              </Row>
            ))}
          </View>
        </View>

        <View
          className="absolute"
          style={{
            left: 0,
            top: 0,
            width: 1920,
            height: 1080,
            visibility: seconds >= TIMES.expand + 0.3 ? "visible" : "hidden",
          }}
        >
          <View
            className="absolute flex items-center justify-between px-12"
            style={{ left: 150, top: 110, width: 1620, height: 140, opacity: detailShown(0) }}
          >
            <Text style={{ fontSize: 72, fontWeight: 900, color: THEME.paper }}>Focus mode</Text>
            <Chip label="ON" fill={THEME.lime} tilt={2} />
          </View>
          <View
            className="absolute flex flex-col items-center justify-center"
            style={{
              ...BOX,
              left: 210,
              top: 315,
              width: 680,
              height: 590,
              backgroundColor: THEME.yellow,
              opacity: detailShown(1),
              translate: `0px ${24 * (1 - detailShown(1))}px`,
            }}
          >
            <Text style={{ fontSize: 230, fontWeight: 900, lineHeight: 1, color: THEME.ink }}>
              25:00
            </Text>
            <Text style={{ fontSize: 28, fontWeight: 900, letterSpacing: 3, color: THEME.ink }}>
              MINUTES OF DEEP WORK
            </Text>
          </View>
          {OPTIONS.map((option, index) => (
            <View
              key={option.id}
              className="absolute flex items-center justify-between px-8"
              style={{
                ...BOX,
                left: 960,
                top: 315 + index * 150,
                width: 760,
                height: 120,
                backgroundColor: THEME.paper,
                opacity: detailShown(index + 2),
                translate: `0px ${24 * (1 - detailShown(index + 2))}px`,
              }}
            >
              <Text style={{ fontSize: 34, fontWeight: 900, color: THEME.ink }}>
                {option.title}
              </Text>
              <Chip label={option.chip} fill={option.fill} />
            </View>
          ))}
          <View
            key="start"
            className="absolute flex items-center justify-center"
            style={{
              ...BOX,
              left: START.x,
              top: START.y,
              width: START.width,
              height: START.height,
              backgroundColor: THEME.pink,
              boxShadow: `${8 - 8 * pressed}px ${8 - 8 * pressed}px 0px ${THEME.ink}`,
              opacity: detailShown(5),
              translate: `${8 * pressed}px ${24 * (1 - detailShown(5)) + 8 * pressed}px`,
            }}
          >
            <Text style={{ fontSize: 42, fontWeight: 900, color: THEME.ink }}>Start session</Text>
          </View>
        </View>
      </View>
    </View>
  );
}

function FocusScreen() {
  const { seconds } = useCurrentTime();
  const { duration } = useComposition();
  // Shapes pop in while the circle opens; text waits until the switch is over.
  const shape = (index) =>
    spring(seconds - TIMES.switchFrom - 0.2 - index * 0.08, { duration: 0.55, bounce: 0.3 });
  const enter = (index) =>
    spring(seconds - TIMES.switchTo - index * 0.07, { duration: 0.55, bounce: 0.3 });
  const elapsed = interpolate(seconds, [TIMES.switchTo, duration], [0.02, 0.14], {
    easing: OUT_SINE,
  });
  return (
    <View
      className="absolute"
      style={{ left: 0, top: 0, width: 1920, height: 1080, backgroundColor: THEME.violet }}
    >
      <View
        className="absolute"
        style={{
          left: 1420,
          top: -170,
          width: 460,
          height: 460,
          borderRadius: 230,
          border: OUTLINE,
          backgroundColor: THEME.yellow,
          scale: 0.6 + 0.4 * shape(0),
        }}
      />
      <View
        className="absolute"
        style={{
          left: -50,
          top: 860,
          width: 220,
          height: 220,
          border: OUTLINE,
          backgroundColor: THEME.lime,
          rotate: `${18 + 12 * seconds}deg`,
          scale: shape(1),
        }}
      />
      <View
        className="absolute"
        style={{
          left: 980,
          top: 930,
          width: 90,
          height: 90,
          borderRadius: 45,
          border: OUTLINE,
          backgroundColor: THEME.pink,
          scale: shape(2),
        }}
      />

      <View
        className="absolute flex flex-col items-start gap-7"
        style={{ left: 140, top: 340, width: 960 }}
      >
        <View style={{ opacity: clamp(enter(0) * 2, 0, 1), scale: 0.5 + 0.5 * enter(0) }}>
          <Chip label="SESSION 1 OF 4" fill={THEME.paper} tilt={-3} />
        </View>
        <Text
          style={{
            fontSize: 156,
            fontWeight: 900,
            lineHeight: 1,
            color: THEME.paper,
            textShadow: `8px 8px 0px ${THEME.ink}`,
            opacity: clamp(enter(1) * 2, 0, 1),
            translate: `0px ${40 * (1 - enter(1))}px`,
          }}
        >
          Deep work
        </Text>
        <Text
          style={{
            width: 920,
            fontSize: 36,
            fontWeight: 700,
            lineHeight: 1.35,
            color: "#F5F3FF",
            opacity: clamp(enter(2) * 2, 0, 1),
            translate: `0px ${24 * (1 - enter(2))}px`,
          }}
        >
          Notifications are off. Just you and the work.
        </Text>
        <View className="flex gap-5" style={{ marginTop: 16 }}>
          {OPTIONS.map((option, index) => (
            <View
              key={`${option.id}-tag`}
              style={{
                opacity: clamp(enter(index + 3) * 2, 0, 1),
                scale: 0.5 + 0.5 * enter(index + 3),
              }}
            >
              <Chip label={option.chip} fill={option.fill} tilt={index === 1 ? 2 : -2} />
            </View>
          ))}
        </View>
      </View>

      <View
        className="absolute flex flex-col items-center justify-center"
        style={{
          ...BOX,
          left: 1190,
          top: 270,
          width: 560,
          height: 560,
          borderRadius: 40,
          backgroundColor: THEME.paper,
          scale: 0.7 + 0.3 * enter(1),
          opacity: clamp(enter(1) * 2, 0, 1),
        }}
      >
        <svg className="absolute" width={550} height={550} style={{ left: 0, top: 0 }}>
          <path d={RING} fill="none" stroke="#EDE9FE" strokeWidth={40} />
          <path
            d={RING}
            fill="none"
            stroke={THEME.pink}
            strokeWidth={40}
            strokeLinecap="round"
            pathLength={1}
            strokeDasharray="1 1"
            strokeDashoffset={1 - elapsed}
          />
        </svg>
        <Text style={{ fontSize: 120, fontWeight: 900, lineHeight: 1, color: THEME.ink }}>
          24:58
        </Text>
        <Text style={{ fontSize: 26, fontWeight: 800, letterSpacing: 3, color: THEME.muted }}>
          REMAINING
        </Text>
      </View>
    </View>
  );
}

// After the switch only the session screen is left. Before it, the home screen paints alone;
// during it, a circle transition from nothing to the session screen opens over the home screen.
function Screens() {
  const { seconds } = useCurrentTime();
  if (seconds >= TIMES.switchTo) return <FocusScreen key="focus" />;
  const progress = interpolate(seconds, [TIMES.switchFrom, TIMES.switchTo], [0, 1], {
    easing: IN_OUT_CUBIC,
  });
  return (
    <>
      <HomeScreen key="home" />
      {seconds >= TIMES.switchFrom && (
        <Transition
          key="switch"
          kind="circleOpen"
          progress={progress}
          params={{
            centerX: (START.x + START.width / 2) / 1920,
            centerY: (START.y + START.height / 2) / 1080,
            softness: 2,
          }}
          style={{ position: "absolute", left: 0, top: 0, width: 1920, height: 1080 }}
        >
          <View key="nothing" />
          <FocusScreen key="focus-in" />
        </Transition>
      )}
    </>
  );
}

function Cursor() {
  const { seconds } = useCurrentTime();
  // Gone once the home screen it points at has given way.
  if (seconds >= TIMES.switchTo) return null;
  const toggle = bounds("screens/home/focus/toggle/track");
  const button = bounds("screens/home/start");
  const park = { x: 1540, y: 1010 };
  const target = { x: toggle.x + toggle.width * 0.7, y: toggle.y + toggle.height * 0.6 };
  const aside = { x: target.x + 70, y: target.y + 120 };
  const press = { x: button.x + button.width * 0.62, y: button.y + button.height * 0.62 };
  let x = park.x;
  let y = park.y;
  if (seconds >= 2.25) {
    const u = interpolate(seconds, [3.25, 3.95], [0, 1], { easing: IN_OUT_SINE });
    const v = interpolate(seconds, [2.25, 2.6], [0, 1], { easing: OUT_CUBIC });
    x = target.x + (aside.x - target.x) * v + (press.x - aside.x) * u;
    y = target.y + (aside.y - target.y) * v + (press.y - aside.y) * u;
  } else {
    const u = interpolate(seconds, [0.85, 1.95], [0, 1], { easing: IN_OUT_SINE });
    x = park.x + (target.x - park.x) * u;
    y = park.y + (target.y - park.y) * u + 60 * Math.sin(u * Math.PI);
  }
  const tap =
    interpolate(seconds, [TIMES.click, TIMES.click + 0.07, TIMES.click + 0.2], [1, 0.82, 1]) *
    interpolate(seconds, [TIMES.press, TIMES.press + 0.07, TIMES.press + 0.2], [1, 0.82, 1]);
  const shown = interpolate(
    seconds,
    [0.6, 0.85, TIMES.switchFrom, TIMES.switchFrom + 0.3],
    [0, 1, 1, 0],
  );
  const ripple = (at) => interpolate(seconds, [at, at + 0.45], [0, 1], { easing: OUT_CUBIC });
  return (
    <View className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080 }}>
      <svg className="absolute inset-0" width={1920} height={1080}>
        {seconds >= TIMES.click && seconds < TIMES.click + 0.45 && (
          <circle
            cx={target.x}
            cy={target.y}
            r={14 + 46 * ripple(TIMES.click)}
            fill="none"
            stroke={THEME.ink}
            strokeWidth={5}
            opacity={1 - ripple(TIMES.click)}
          />
        )}
        {seconds >= TIMES.press && seconds < TIMES.press + 0.45 && (
          <circle
            cx={press.x}
            cy={press.y}
            r={14 + 46 * ripple(TIMES.press)}
            fill="none"
            stroke={THEME.ink}
            strokeWidth={5}
            opacity={1 - ripple(TIMES.press)}
          />
        )}
      </svg>
      <View
        className="absolute"
        style={{
          left: 0,
          top: 0,
          width: 50,
          height: 70,
          opacity: shown,
          translate: `${x}px ${y}px`,
          scale: tap,
          transformOrigin: "0px 0px",
        }}
      >
        <svg width={50} height={70} style={{ overflow: "visible" }}>
          <path d={ARROW} fill={THEME.ink} transform="translate(5 5)" />
          <path
            d={ARROW}
            fill={THEME.paper}
            stroke={THEME.ink}
            strokeWidth={4}
            strokeLinejoin="round"
          />
        </svg>
      </View>
    </View>
  );
}

export default function UiShowcase() {
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: THEME.lime }}>
      <Screens key="screens" />
      <Cursor key="cursor" />
    </View>
  );
}
