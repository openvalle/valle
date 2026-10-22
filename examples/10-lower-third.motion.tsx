// Lower third: a name strap unfolds from a slanted spine, holds with a breathing LIVE dot and folds
// back out. Shows overlay({ intro, outro, hold }), controls.props, defineSequence cue sheets,
// yoyoProgress, textTransform, and the clip clock (clipSeconds, clipDuration) beside the looped
// middle.
// Render: valle motion render examples/10-lower-third.motion.tsx -o 10-lower-third.mov
// Timeline: add it to "resources" as "lowerThird", then place a clip whose duration sets the hold:
//   { "kind": "motion", "component": "lowerThird", "start": 2, "duration": 9, "props": { "name": "Ada Park" } }

// Seconds: the strap is in after INTRO, starts to leave OUTRO before the end, and repeats the
// middle between them on a longer clip.
const DURATION = 6;
const INTRO = 1.2;
const OUTRO = 0.8;
const OUTRO_START = DURATION - OUTRO;
// Laid out for a 1080p host and scaled to others.
const WIDTH = 1920;
const HEIGHT = 1080;

export const composition = { width: WIDTH, height: HEIGHT, fps: 60, duration: DURATION };

// The dot keeps breathing for as long as the strap is on.
export const role = overlay({ intro: INTRO, outro: OUTRO, hold: "loop" });

export const controls = {
  props: {
    name: string({ default: "Maya Okafor", label: "Name" }),
    title: string({ default: "Head of Design · Valle Studio", label: "Title" }),
    accent: color({ default: "#FF3D7F", label: "Name bar" }),
    highlight: color({ default: "#FFC93C", label: "Title bar" }),
  },
};

const INK = "#2B2D42";
const SHADOW = "0px 6px 0px #2B2D4233";
// The strap leans forward; its text leans back upright.
const SLANT = "skewX(-12deg)";
const UPRIGHT = "skewX(12deg)";

// The bars unfold one after another, then the words land on them; they leave in reverse.
const ENTER = defineSequence({
  spine: stage({ at: 0, duration: 0.3 }),
  nameBar: stage({ at: 0.1, duration: 0.5 }),
  titleBar: stage({ at: 0.24, duration: 0.5 }),
  liveBar: stage({ at: 0.34, duration: 0.45 }),
  name: stage({ at: 0.44, duration: 0.5 }),
  title: stage({ at: 0.54, duration: 0.45 }),
  live: stage({ at: 0.68, duration: 0.4 }),
});
const LEAVE = defineSequence({
  live: stage({ at: OUTRO_START, duration: 0.22 }),
  title: stage({ at: OUTRO_START + 0.02, duration: 0.28 }),
  name: stage({ at: OUTRO_START + 0.06, duration: 0.28 }),
  liveBar: stage({ at: OUTRO_START + 0.16, duration: 0.26 }),
  titleBar: stage({ at: OUTRO_START + 0.2, duration: 0.28 }),
  nameBar: stage({ at: OUTRO_START + 0.36, duration: 0.3 }),
  spine: stage({ at: OUTRO_START + 0.5, duration: 0.26 }),
});
// The middle that repeats: four breaths of the live dot and one sheen across the name bar.
const HOLD = defineSequence({
  breath: stage({ at: INTRO, duration: OUTRO_START - INTRO }),
  sheen: stage({ at: INTRO + 0.35, duration: 1.1 }),
});

// How far a layer is in: eased in over its entrance stage, eased out over its exit stage.
function shown(layer, enter, leave) {
  const { seconds } = useCurrentTime();
  return interpolate(stageProgress(seconds, ENTER[layer]), [0, 1], [0, 1], { easing: enter }) -
    interpolate(stageProgress(seconds, LEAVE[layer]), [0, 1], [0, 1], { easing: leave });
}

// A bar of the strap, unfolding from its left edge behind the content laid over it.
function Bar({ fill, open, children }) {
  return (
    <View key="bar" className="absolute inset-0" style={{ backgroundColor: fill, boxShadow: SHADOW,
      transformOrigin: "0% 50%", transform: `scaleX(${open})` }}>
      {children}
    </View>
  );
}

function LiveTag({ accent, calm }) {
  const { seconds } = useCurrentTime();
  const lit = shown("live", "easeOutCubic", "easeInCubic");
  // Eight swings, an even count, end where they start, so the repeated middle joins seamlessly.
  const breath = calm *
    interpolate(yoyoProgress(seconds, HOLD.breath, 8), [0, 1], [0, 1], { easing: "easeInOutSine" });
  return (
    <View key="live" className="relative flex items-center" style={{ padding: "0px 26px" }}>
      <Bar key="live-bar" fill={INK} open={shown("liveBar", "easeOutExpo", "easeInCubic")} />
      <View key="live-copy" className="flex items-center" style={{ gap: 14, transform: UPRIGHT,
        translate: point(-18 * (1 - lit), 0), opacity: lit }}>
        <View key="dot" className="relative" style={{ width: 16, height: 16 }}>
          <View key="halo" className="absolute inset-0 rounded-full"
            style={{ backgroundColor: accent, opacity: 0.45, scale: 1 + 1.4 * breath }} />
          <View key="core" className="absolute inset-0 rounded-full"
            style={{ backgroundColor: accent, scale: 1 + 0.15 * breath }} />
        </View>
        <Text key="live-label" style={{ fontSize: 24, fontWeight: 800, letterSpacing: 3, color: "#FFFFFF" }}>
          LIVE
        </Text>
      </View>
    </View>
  );
}

function NameLine({ name, accent, calm }) {
  const { seconds } = useCurrentTime();
  const drop = shown("name", "easeOutCubic", "easeInCubic");
  // The sheen, 30% of the bar wide, crosses from beyond its left edge to beyond its right.
  const sweep = interpolate(stageProgress(seconds, HOLD.sheen), [0, 1], [-120, 360],
    { easing: "easeInOutQuad" });
  return (
    <View key="name-line" className="flex items-stretch" style={{ gap: 10 }}>
      <View key="name-box" className="relative">
        <Bar key="name-bar" fill={accent} open={shown("nameBar", "easeOutExpo", "easeInCubic")} />
        <View key="sheen-clip" className="absolute inset-0 overflow-hidden">
          <View key="sheen" className="absolute top-0 bottom-0 left-0" style={{ width: "30%",
            backgroundImage: "linear-gradient(90deg, #FFFFFF00, #FFFFFF66, #FFFFFF00)",
            translate: `${sweep}% 0%`, opacity: calm }} />
        </View>
        {/* A flex box makes the text a block, so it can move; inline text, as in CSS, cannot. */}
        <View key="name-mask" className="flex overflow-hidden" style={{ padding: "10px 60px 12px 46px",
          transform: UPRIGHT }}>
          <Text key="name" style={{ fontSize: 76, lineHeight: 1.1, color: "#FFFFFF",
            fontVariationSettings: '"wght" 800, "wdth" 84',
            translate: `0% ${-110 * (1 - drop)}%`, opacity: clamp(5 * drop, 0, 1) }}>
            {name}
          </Text>
        </View>
      </View>
      <LiveTag accent={accent} calm={calm} />
    </View>
  );
}

function TitleLine({ title, highlight, onAir }) {
  const slide = shown("title", "easeOutCubic", "easeInCubic");
  return (
    <View key="title-box" className="relative" style={{ marginLeft: 30 }}>
      <Bar key="title-bar" fill={highlight} open={shown("titleBar", "easeOutExpo", "easeInCubic")}>
        <View key="on-air" className="absolute bottom-0 left-0 right-0" style={{ height: 5,
          backgroundColor: "#2B2D4247", transformOrigin: "0% 50%", scale: point(onAir, 1) }} />
      </Bar>
      <View key="title-mask" className="flex overflow-hidden" style={{ padding: "13px 34px 14px 30px",
        transform: UPRIGHT }}>
        <Text key="title" style={{ fontSize: 27, lineHeight: 1.2, letterSpacing: 3.5, color: INK,
          textTransform: "uppercase", fontVariationSettings: '"wght" 720, "wdth" 92',
          translate: `${-36 * (1 - slide)}% 0%`, opacity: clamp(5 * slide, 0, 1) }}>
          {title}
        </Text>
      </View>
    </View>
  );
}

export default function LowerThird({ name, title, accent, highlight }) {
  const { clipSeconds } = useCurrentTime();
  const { width, height, clipDuration } = useComposition();
  // The strap scales with the host instead of laying out again, so its text is measured once.
  const fit = Math.min(width / WIDTH, height / HEIGHT);
  // `seconds` repeats the middle on a long clip, while the clip clock runs straight through: it
  // fills the on-air line by the outro and calms the loops just before the outro cuts in.
  const onAir = clamp((clipSeconds - INTRO) / Math.max(clipDuration - INTRO - OUTRO, 0.1), 0, 1);
  const calm = clamp((clipDuration - OUTRO - clipSeconds) / 0.3, 0, 1);
  const spine = shown("spine", "easeOutCubic", "easeInCubic");
  return (
    <Scene className="relative h-full w-full">
      <View key="safe-area" className="absolute bottom-0 left-0"
        style={{ padding: "0px 0px 128px 136px", transformOrigin: "0% 100%", scale: fit }}>
        <View key="strap" className="flex items-stretch" style={{ gap: 12, transform: SLANT }}>
          <View key="spine" style={{ width: 14, backgroundColor: INK, transform: `scaleY(${spine})` }} />
          <View key="lines" className="flex flex-col items-start" style={{ gap: 10 }}>
            <NameLine name={name} accent={accent} calm={calm} />
            <TitleLine title={title} highlight={highlight} onAir={onAir} />
          </View>
        </View>
      </View>
    </Scene>
  );
}
