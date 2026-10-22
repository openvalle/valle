// Logo sting: a circle, a square and a triangle spin in, change shape on their way into one
// Bauhaus mark, an outline draws around it and the wordmark slides out from behind it.
// Shows path(), arc(), sector(), morphPath(), roundCorners(), boolean() and offsetPath(),
// trimStart/trimEnd with strokeLinecap, Clip, Circle/Rect/Line/Polygon, transformOrigin,
// cubic-bezier() easings and a TimeScope that replays one component at three moments.
// Render: valle motion render examples/02-logo-sting.motion.tsx -o 02-logo-sting.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

const PAPER = "#F4F1EA";
const RED = "#E63946";
const SAND = "#F4A261";
const TEAL = "#2A9D8F";
const NAVY = "#1D3557";

const SWIFT = "cubic-bezier(0.16, 1, 0.3, 1)";
const GLIDE = "cubic-bezier(0.65, 0, 0.35, 1)";
const POP = "cubic-bezier(0.34, 1.56, 0.64, 1)";

// Beats of the sting, in seconds, and how far the finished lockup slides to make room.
const GATHER = 1.35;
const LOCK = 2.15;
const SLIDE = 3.15;
const SHIFT = -440;

// The mark is a 360 x 360 tile at the centre of the canvas: a teal arch over a red square and a
// sand ramp. Each piece is a box drawn in its own coordinates, so it turns and scales about its
// centre as in CSS. morphPath() pairs the commands of two paths one by one: the circle and the
// arch are four cubics each, so the lower half of the circle flattens into the arch's base, and
// the two triangles share their three lines.
const CIRCLE = path("M 0 150 C 0 67.16 67.16 0 150 0 C 232.84 0 300 67.16 300 150 C 300 232.84 232.84 300 150 300 C 67.16 300 0 232.84 0 150 Z");
const ARCH = path("M -30 150 C -30 50.59 50.59 -30 150 -30 C 249.41 -30 330 50.59 330 150 C 270 150 210 150 150 150 C 90 150 30 150 -30 150 Z");
const SQUARE = path("M 0 0 L 180 0 L 180 180 L 0 180 Z");
const TRIANGLE = path("M 155 0 L 310 268 L 0 268 Z");
const RAMP = path("M 95 59 L 275 239 L 95 239 Z");

// The outline runs 24 px outside the union of the pieces where they lock.
const OUTLINE = offsetPath(
  boolean(
    boolean(
      sector({ center: point(960, 540), outer: 180, start: deg(180), end: deg(360) }),
      path("M 780 540 L 960 540 L 960 720 L 780 720 Z"),
      "union",
    ),
    path("M 960 540 L 1140 720 L 960 720 Z"),
    "union",
  ),
  -24,
);
const WORD_WINDOW = path("M 1172 300 L 2400 300 L 2400 590 L 1172 590 Z");
const TAG_WINDOW = path("M 1172 610 L 2400 610 L 2400 720 L 1172 720 Z");
const CORNER_RING = arc(point(0, 1080), 260, deg(-90), deg(0));

const DOTS = Array.from({ length: 12 }, (_, i) => ({
  id: `dot-${i}`,
  x: 1622 + (i % 4) * 56,
  y: 874 + Math.floor(i / 4) * 56,
  delay: 0.5 + i * 0.04,
}));

// Pieces flying sideways lift on the way, as a hop.
function hop(glide) {
  return -100 * sin(glide * deg(180));
}

// One ring that spreads and fades over a second; each TimeScope starts it at its own moment.
function Ripple({ color }) {
  const { seconds } = useCurrentTime();
  const grow = interpolate(seconds, [0, 1.2], [0, 1], { easing: "easeOutCubic" });
  // The same colour with an alpha of 00 fades the ring out without darkening it.
  const clear = `${color}00`;
  return (
    <Circle cx={960} cy={540} r={250 + grow * 380} fill="none" strokeWidth={16 - grow * 13}
      stroke={interpolate(seconds, [0, 0.1, 1.2], [clear, color, clear])} />
  );
}

export default function LogoSting() {
  const { seconds } = useCurrentTime();

  // Construction lines grow out from the centre, then give way to the mark.
  const guide = interpolate(seconds, [0, 0.9], [0, 1], { easing: SWIFT });
  const guideInk = interpolate(seconds, [GATHER, LOCK], ["#1D355740", "#1D355700"]);

  // The line-up: each primitive pops in with its own entrance.
  const circleIn = interpolate(seconds, [0.15, 0.85], [0, 1], { easing: POP });
  const squareIn = interpolate(seconds, [0.25, 0.95], [0, 1], { easing: POP });
  const triangleIn = interpolate(seconds, [0.35, 1.05], [0, 1], { easing: POP });
  const squareSpin = interpolate(seconds, [0.25, 1.15], [-270, -90], { easing: SWIFT });
  const triangleSpin = interpolate(seconds, [0.35, 1.25], [300, 120], { easing: SWIFT });

  // The gather: every piece glides to its slot and takes its final shape on the way. The square
  // starts a quarter turn back and the triangle a third of a turn, so both land upright.
  const archGlide = interpolate(seconds, [GATHER, LOCK], [0, 1], { easing: GLIDE });
  const squareGlide = interpolate(seconds, [GATHER + 0.05, LOCK], [0, 1], { easing: GLIDE });
  const rampGlide = interpolate(seconds, [GATHER + 0.1, LOCK], [0, 1], { easing: GLIDE });

  // The lock: the mark swells once while the ripples spread and the outline draws.
  const swell = interpolate(seconds, [LOCK, LOCK + 0.16, LOCK + 0.6], [1, 1.05, 1],
    { easing: ["easeOutCubic", "easeInOutSine"] });
  const draw = interpolate(seconds, [LOCK + 0.1, LOCK + 1.0], [0, 1], { easing: GLIDE });
  const lead = interpolate(seconds, [LOCK + 0.22, LOCK + 1.1], [0, 1], { easing: GLIDE });
  const leadInk = interpolate(seconds, [LOCK + 0.85, LOCK + 1.1], [RED, "#E6394600"]);

  // The lockup slides left while the wordmark comes out from behind the mark.
  const slide = interpolate(seconds, [SLIDE, SLIDE + 0.8], [0, SHIFT], { easing: GLIDE });
  const word = interpolate(seconds, [SLIDE + 0.1, SLIDE + 1.0], [0, 1], { easing: SWIFT });
  const rule = interpolate(seconds, [SLIDE + 0.35, SLIDE + 1.15], [0, 1], { easing: SWIFT });
  const tagline = interpolate(seconds, [SLIDE + 0.45, SLIDE + 1.2], [0, 1], { easing: SWIFT });
  const period = spring(seconds - SLIDE - 1.05, { duration: 0.5, bounce: 0.45 });

  // Corner accents, and the idle motion that keeps the held end alive.
  const sweep = interpolate(seconds, [0.1, 1.0], [0.01, 1], { easing: SWIFT });
  const ring = interpolate(seconds, [0.3, 1.3], [0, 1], { easing: SWIFT });
  const badge = interpolate(seconds, [0.5, 1.1], [0, 1], { easing: POP });
  const breathe = sin(seconds * deg(150));

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: PAPER }}>
      <Path key="corner-sector" fill={RED}
        d={sector({ center: point(1920, 0), outer: 320 + breathe * 6, start: deg(90), end: deg(90 + 90 * sweep) })} />
      <Path key="corner-ring" d={CORNER_RING} fill="none" stroke={TEAL} strokeWidth={34}
        strokeDasharray="40 24" strokeDashoffset={-seconds * 40} trimEnd={ring} />
      <Polygon key="corner-triangle" className="absolute" fill={SAND}
        points={[point(50, 0), point(100, 87), point(0, 87)]}
        style={{ left: 180, top: 150, width: 100, height: 87, transformOrigin: point(50, 58),
          rotate: `${seconds * 14 - 30}deg`, scale: badge }} />
      {DOTS.map((dot) => (
        <Circle key={dot.id} className="absolute" cx={9} cy={9} r={9} fill={NAVY}
          style={{ left: dot.x - 9, top: dot.y - 9, width: 18, height: 18,
            scale: interpolate(seconds, [dot.delay, dot.delay + 0.5], [0, 1], { easing: POP }) }} />
      ))}

      <Line key="guide-x" x1={0} y1={540} x2={1920} y2={540} fill="none" stroke={guideInk}
        strokeWidth={2} trimStart={0.5 - guide * 0.5} trimEnd={0.5 + guide * 0.5} />
      <Line key="guide-y" x1={960} y1={0} x2={960} y2={1080} fill="none" stroke={guideInk}
        strokeWidth={2} trimStart={0.5 - guide * 0.5} trimEnd={0.5 + guide * 0.5} />

      <View key="lockup" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080,
        translate: point(slide, 0) }}>
        <TimeScope key="ripple-teal" offset={LOCK} speed={1}>
          <Ripple key="ring-teal" color={TEAL} />
        </TimeScope>
        <TimeScope key="ripple-red" offset={LOCK + 0.12} speed={1}>
          <Ripple key="ring-red" color={RED} />
        </TimeScope>
        <TimeScope key="ripple-sand" offset={LOCK + 0.24} speed={1}>
          <Ripple key="ring-sand" color={SAND} />
        </TimeScope>

        <View key="mark" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080,
          scale: swell, transformOrigin: point(960, 540) }}>
          <Path key="arch" className="absolute" d={morphPath(CIRCLE, ARCH, archGlide)} fill={TEAL}
            style={{ left: 0, top: 0, width: 300, height: 300, scale: circleIn,
              translate: point(270 + 540 * archGlide, 390 + 60 * (1 - circleIn) + hop(archGlide)) }} />
          <Path key="square" className="absolute" d={roundCorners(SQUARE, 34 * (1 - squareGlide))} fill={RED}
            style={{ left: 0, top: 0, width: 180, height: 180,
              translate: point(870 - 90 * squareGlide, 450 + 90 * squareGlide),
              rotate: `${squareSpin + 90 * squareGlide}deg`, scale: squareIn * (1.667 - 0.667 * squareGlide) }} />
          <Path key="ramp" className="absolute" d={morphPath(TRIANGLE, RAMP, rampGlide)} fill={SAND}
            style={{ left: 0, top: 0, width: 310, height: 268, transformOrigin: point(155, 179),
              translate: point(1345 - 480 * rampGlide, 406 + 75 * rampGlide + hop(rampGlide)),
              rotate: `${triangleSpin - 120 * rampGlide}deg`, scale: triangleIn }} />
          <Path key="outline" d={OUTLINE} fill="none" stroke={NAVY} strokeWidth={6}
            strokeLinecap="round" trimEnd={draw} visible={seconds > LOCK + 0.1} />
          <Path key="outline-lead" d={OUTLINE} fill="none" stroke={leadInk} strokeWidth={12}
            strokeLinecap="round" trimStart={lead} trimEnd={draw} visible={seconds > LOCK + 0.1} />
        </View>

        <Clip key="word-window" path={WORD_WINDOW} style={{ position: "absolute", left: 0, top: 0,
          width: 1920, height: 1080 }}>
          <Text key="wordmark" className="absolute" style={{ left: 1212, top: 338, fontSize: 220,
            fontWeight: 800, letterSpacing: -5, lineHeight: 1, color: NAVY,
            translate: point(-900 * (1 - word), 0) }}>FORMA</Text>
        </Clip>
        <Rect key="rule" x={1222} y={600} width={766 * rule} height={6} fill={NAVY} />
        <Clip key="tag-window" path={TAG_WINDOW} style={{ position: "absolute", left: 0, top: 0,
          width: 1920, height: 1080 }}>
          <Text key="tagline" className="absolute" visible={seconds > SLIDE + 0.45}
            style={{ left: 1220, top: 640, fontSize: 44, fontWeight: 700, letterSpacing: 12,
              color: TEAL, translate: point(0, 100 * (1 - tagline)) }}>FORM FOLLOWS MOTION</Text>
        </Clip>
        <Circle key="period" className="absolute" cx={26} cy={26} r={26} fill={RED}
          style={{ left: 2014, top: 480, width: 52, height: 52, scale: period * (1 + breathe * 0.06) }} />
      </View>
    </Scene>
  );
}
