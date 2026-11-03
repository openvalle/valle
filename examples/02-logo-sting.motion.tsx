// Logo sting: a circle, a square and a triangle spin in, change shape on their way into one
// Bauhaus mark, an outline draws around it and the wordmark slides out from behind it.
// Shows inline SVG paths, d3's arc(), morph(), roundCorners(), pathBoolean() and inflatePath(),
// strokes drawn with pathLength and dashes, windows clipped with overflow, transformOrigin,
// cubic-bezier() easings and a TimeScope that replays one component at three moments.
// Render: valle motion render examples/02-logo-sting.motion.tsx -o 02-logo-sting.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

// Easings.
const IN_OUT_SINE = "cubic-bezier(0.37, 0, 0.63, 1)";
const OUT_CUBIC = "cubic-bezier(0.33, 1, 0.68, 1)";

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
// sand ramp. Each piece is an SVG box drawn in its own coordinates, so it turns and scales about
// its centre as in CSS. morph() pairs the outlines of two paths, so the lower half of the circle
// flattens into the arch's base and the triangle leans into the ramp.
const CIRCLE =
  "M 0 150 C 0 67.16 67.16 0 150 0 C 232.84 0 300 67.16 300 150 C 300 232.84 232.84 300 150 300 C 67.16 300 0 232.84 0 150 Z";
const ARCH =
  "M -30 150 C -30 50.59 50.59 -30 150 -30 C 249.41 -30 330 50.59 330 150 C 270 150 210 150 150 150 C 90 150 30 150 -30 150 Z";
const SQUARE = "M 0 0 L 180 0 L 180 180 L 0 180 Z";
const TRIANGLE = "M 155 0 L 310 268 L 0 268 Z";
const RAMP = "M 95 59 L 275 239 L 95 239 Z";

// The outline runs 24 px outside the union of the pieces where they lock.
const OUTLINE = inflatePath(
  pathBoolean(
    pathBoolean(
      "M 780 540 A 180 180 0 0 1 1140 540 Z",
      "M 780 540 L 960 540 L 960 720 L 780 720 Z",
      "union",
    ),
    "M 960 540 L 1140 720 L 960 720 Z",
    "union",
  ),
  24,
);
const CORNER_RING = "M 0 820 A 260 260 0 0 1 260 1080";

const DOTS = Array.from({ length: 12 }, (_, i) => ({
  id: `dot-${i}`,
  x: 1622 + (i % 4) * 56,
  y: 874 + Math.floor(i / 4) * 56,
  delay: 0.5 + i * 0.04,
}));

// Pieces flying sideways lift on the way, as a hop.
function hop(glide) {
  return -100 * Math.sin(glide * Math.PI);
}

// One ring that spreads and fades over a second; each TimeScope starts it at its own moment.
function Ripple({ color }) {
  const { seconds } = useCurrentTime();
  const grow = interpolate(seconds, [0, 1.2], [0, 1], { easing: OUT_CUBIC });
  // The same colour with an alpha of 00 fades the ring out without darkening it.
  const clear = `${color}00`;
  return (
    <svg className="absolute inset-0" width={1920} height={1080}>
      <circle
        cx={960}
        cy={540}
        r={250 + grow * 380}
        fill="none"
        strokeWidth={16 - grow * 13}
        stroke={interpolate(seconds, [0, 0.1, 1.2], [clear, color, clear])}
      />
    </svg>
  );
}

// A piece of the mark: an SVG box placed by its transform.
function Piece({ width, height, d, fill, style }) {
  return (
    <svg
      className="absolute"
      width={width}
      height={height}
      style={{ left: 0, top: 0, overflow: "visible", ...style }}
    >
      <path d={d} fill={fill} />
    </svg>
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

  // The lock: the mark swells once while the ripples spread and the outline draws, a red lead
  // running ahead of it.
  const swell =
    1 +
    0.05 * interpolate(seconds, [LOCK, LOCK + 0.16], [0, 1], { easing: OUT_CUBIC }) -
    0.05 * interpolate(seconds, [LOCK + 0.16, LOCK + 0.6], [0, 1], { easing: IN_OUT_SINE });
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
  const breathe = Math.sin((seconds * 150 * Math.PI) / 180);
  const corner = arc()
    .innerRadius(0)
    .outerRadius(320 + breathe * 6)
    .startAngle(Math.PI)
    .endAngle(Math.PI + (Math.PI / 2) * sweep);
  // The ring's dashes march along it while a growing quarter disc uncovers it.
  const uncover = arc()
    .innerRadius(0)
    .outerRadius(400)
    .startAngle(0)
    .endAngle((Math.PI / 2) * ring);

  return (
    <View className="relative h-full w-full" style={{ backgroundColor: PAPER }}>
      <svg className="absolute inset-0" width={1920} height={1080}>
        <defs>
          <clipPath id="ring-reveal">
            <path d={uncover()} transform="translate(0 1080)" />
          </clipPath>
        </defs>
        <path d={corner()} fill={RED} transform="translate(1920 0)" />
        <path
          d={CORNER_RING}
          fill="none"
          stroke={TEAL}
          strokeWidth={34}
          strokeDasharray="40 24"
          strokeDashoffset={-seconds * 40}
          clipPath="url(#ring-reveal)"
        />
        <line
          x1={960 * (1 - guide)}
          y1={540}
          x2={960 * (1 + guide)}
          y2={540}
          stroke={guideInk}
          strokeWidth={2}
        />
        <line
          x1={960}
          y1={540 * (1 - guide)}
          x2={960}
          y2={540 * (1 + guide)}
          stroke={guideInk}
          strokeWidth={2}
        />
      </svg>
      <svg
        key="corner-triangle"
        className="absolute"
        width={100}
        height={87}
        style={{
          left: 180,
          top: 150,
          transformOrigin: "50px 58px",
          rotate: `${seconds * 14 - 30}deg`,
          scale: badge,
        }}
      >
        <polygon points="50,0 100,87 0,87" fill={SAND} />
      </svg>
      {DOTS.map((dot) => (
        <View
          key={dot.id}
          className="absolute rounded-full"
          style={{
            left: dot.x - 9,
            top: dot.y - 9,
            width: 18,
            height: 18,
            backgroundColor: NAVY,
            scale: interpolate(seconds, [dot.delay, dot.delay + 0.5], [0, 1], { easing: POP }),
          }}
        />
      ))}

      <View
        key="lockup"
        className="absolute"
        style={{ left: 0, top: 0, width: 1920, height: 1080, translate: `${slide}px 0px` }}
      >
        <TimeScope key="ripple-teal" offset={LOCK}>
          <Ripple color={TEAL} />
        </TimeScope>
        <TimeScope key="ripple-red" offset={LOCK + 0.12}>
          <Ripple color={RED} />
        </TimeScope>
        <TimeScope key="ripple-sand" offset={LOCK + 0.24}>
          <Ripple color={SAND} />
        </TimeScope>

        <View
          key="mark"
          className="absolute"
          style={{
            left: 0,
            top: 0,
            width: 1920,
            height: 1080,
            scale: swell,
            transformOrigin: "960px 540px",
          }}
        >
          <Piece
            key="arch"
            width={300}
            height={300}
            d={morph(CIRCLE, ARCH, archGlide)}
            fill={TEAL}
            style={{
              scale: circleIn,
              translate: `${270 + 540 * archGlide}px ${390 + 60 * (1 - circleIn) + hop(archGlide)}px`,
            }}
          />
          <Piece
            key="square"
            width={180}
            height={180}
            d={roundCorners(SQUARE, 34 * (1 - squareGlide))}
            fill={RED}
            style={{
              translate: `${870 - 90 * squareGlide}px ${450 + 90 * squareGlide}px`,
              rotate: `${squareSpin + 90 * squareGlide}deg`,
              scale: squareIn * (1.667 - 0.667 * squareGlide),
            }}
          />
          <Piece
            key="ramp"
            width={310}
            height={268}
            d={morph(TRIANGLE, RAMP, rampGlide)}
            fill={SAND}
            style={{
              transformOrigin: "155px 179px",
              translate: `${1345 - 480 * rampGlide}px ${406 + 75 * rampGlide + hop(rampGlide)}px`,
              rotate: `${triangleSpin - 120 * rampGlide}deg`,
              scale: triangleIn,
            }}
          />
          {seconds > LOCK + 0.1 && (
            <svg key="outline" className="absolute inset-0" width={1920} height={1080}>
              <path
                d={OUTLINE}
                fill="none"
                stroke={NAVY}
                strokeWidth={6}
                strokeLinecap="round"
                pathLength={1}
                strokeDasharray="1 1"
                strokeDashoffset={1 - draw}
              />
              {draw > lead && (
                <path
                  d={OUTLINE}
                  fill="none"
                  stroke={leadInk}
                  strokeWidth={12}
                  strokeLinecap="round"
                  pathLength={1}
                  strokeDasharray={`${draw - lead} 2`}
                  strokeDashoffset={-lead}
                />
              )}
            </svg>
          )}
        </View>

        <View
          key="word-window"
          className="absolute overflow-hidden"
          style={{ left: 1172, top: 300, width: 1228, height: 290 }}
        >
          <Text
            key="wordmark"
            className="absolute"
            style={{
              left: 40,
              top: 38,
              fontSize: 220,
              fontWeight: 800,
              letterSpacing: -5,
              lineHeight: 1,
              color: NAVY,
              whiteSpace: "nowrap",
              translate: `${-900 * (1 - word)}px 0px`,
            }}
          >
            FORMA
          </Text>
        </View>
        <View
          key="rule"
          className="absolute"
          style={{ left: 1222, top: 600, width: 766 * rule, height: 6, backgroundColor: NAVY }}
        />
        <View
          key="tag-window"
          className="absolute overflow-hidden"
          style={{ left: 1172, top: 610, width: 1228, height: 110 }}
        >
          {seconds > SLIDE + 0.45 && (
            <Text
              key="tagline"
              className="absolute"
              style={{
                left: 48,
                top: 30,
                fontSize: 44,
                fontWeight: 700,
                letterSpacing: 12,
                color: TEAL,
                whiteSpace: "nowrap",
                translate: `0px ${100 * (1 - tagline)}px`,
              }}
            >
              FORM FOLLOWS MOTION
            </Text>
          )}
        </View>
        <View
          key="period"
          className="absolute rounded-full"
          style={{
            left: 2014,
            top: 480,
            width: 52,
            height: 52,
            backgroundColor: RED,
            scale: period * (1 + breathe * 0.06),
          }}
        />
      </View>
    </View>
  );
}
