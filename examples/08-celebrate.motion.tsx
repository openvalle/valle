// Celebrate: a bouncing title, confetti cannons, a medal on its ribbon and a shooting star.
// Shows particles() with curlNoise and drag rendered with .map, field(), simulate(), an elastic
// linear() easing, springs, wiggle, CSS offsetPath, and three bounded effects: a 4-sample Shutter
// on the medal, an Echo trail and motionBlur "auto" on the star.
// Render: valle motion render examples/08-celebrate.motion.tsx -o 08-celebrate.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

// Easings.
const IN_OUT_CUBIC = "cubic-bezier(0.65, 0, 0.35, 1)";
const IN_SINE = "cubic-bezier(0.12, 0, 0.39, 0)";
const OUT_BACK = "cubic-bezier(0.34, 1.56, 0.64, 1)";
const OUT_CUBIC = "cubic-bezier(0.33, 1, 0.68, 1)";
const OUT_ELASTIC =
  "linear(0, 0.272, 0.646, 1, 1.25, 1.364, 1.354, 1.257, 1.125, 1, 0.912, 0.871, 0.875, 0.909, 0.956, 1, 1.031, 1.046, 1.044, 1.032, 1.016, 1, 0.989, 0.984, 0.984, 0.989, 0.994, 1, 1.004, 1.006, 1.006, 1.004, 1.002, 1, 0.999, 0.998, 0.998, 0.999, 0.999, 1, 1)";
const OUT_EXPO = "cubic-bezier(0.16, 1, 0.3, 1)";

const INK = "#FFFFFF";
const SHADE = "#4A3AC4";
const CONFETTI = ["#FFD166", "#EF476F", "#06D6A0", "#118AB2", "#FFFFFF"];

const BURST = 0.45;
const MEDAL_IN = 1.0;
const STAR_FLIGHT = [3.0, 3.65];
const MEDAL_CENTER = { x: 960, y: 330 };

function starPath(outer: number, inner: number) {
  const corners = Array.from({ length: 10 }, (_, i) => {
    const r = i % 2 === 0 ? outer : inner;
    const a = -Math.PI / 2 + (i * Math.PI) / 5;
    return `${i === 0 ? "M" : "L"} ${(r * Math.cos(a)).toFixed(2)} ${(r * Math.sin(a)).toFixed(2)}`;
  });
  return `${corners.join(" ")} Z`;
}

// 18 thin wedges around (0, 0), long enough to reach past every corner of the canvas.
const RAYS = Array.from({ length: 18 }, (_, i) => {
  const a = (i / 18) * Math.PI * 2;
  const b = a + Math.PI / 18;
  const r = 1900;
  return `M 0 0 L ${(r * Math.cos(a)).toFixed(1)} ${(r * Math.sin(a)).toFixed(1)} L ${(r * Math.cos(b)).toFixed(1)} ${(r * Math.sin(b)).toFixed(1)} Z`;
}).join(" ");
const EMBLEM = roundCorners(starPath(70, 30), 6);
const STAR = roundCorners(starPath(30, 13), 3);
const TAIL = "M 2 -12 Q -120 -5 -380 0 Q -120 5 2 12 Z";
const STAR_ARC = "M -260 120 Q 900 10 2180 300";
const SPARKLE = "M 0 -1 Q 0.12 -0.12 1 0 Q 0.12 0.12 0 1 Q -0.12 0.12 -1 0 Q -0.12 -0.12 0 -1 Z";

// One physical model, baked once at prepare time: a ribbon spring catches the fall, a weak
// restoring torque captures the spin facing front, and the catch leaves a damped swing.
const MEDAL = simulate({
  dt: 1 / 240,
  duration: 5.2,
  init: () => ({ drop: -760, dropV: 0, spin: 0, spinV: 26, swing: 7, swingV: 0 }),
  step: (s, t, dt) => {
    const dropV = s.dropV + (-40 * s.drop - 7.5 * s.dropV) * dt;
    const spinV = s.spinV + (-30 * Math.sin(s.spin) - 1.4 * s.spinV) * dt;
    const swingV = s.swingV + (-10 * s.swing - 1.1 * s.swingV) * dt;
    return {
      drop: s.drop + dropV * dt,
      dropV,
      spin: s.spin + spinV * dt,
      spinV,
      swing: s.swing + swingV * dt,
      swingV,
    };
  },
});

// Confetti: three spin families per cannon. Each piece has its own color and shape, and turns
// and leans from the first to the second value over its life.
const PIECES = 72;
const PIECE_COLORS = Array.from({ length: PIECES }, (_, i) => CONFETTI[(i * 3) % CONFETTI.length]);
const STRIPS = Array.from({ length: PIECES }, (_, i) => (i % 3 === 0 ? [17, 17] : [30, 12]));
const DOTS = Array.from({ length: PIECES }, (_, i) => (i % 2 === 0 ? [15, 15] : [11, 11]));
const FAMILIES = [
  { id: "strips", round: false, shapes: STRIPS, seed: 11, spin: [0, 2100], skew: [-50, 40] },
  { id: "flips", round: false, shapes: STRIPS, seed: 23, spin: [140, -1700], skew: [45, -55] },
  { id: "dots", round: true, shapes: DOTS, seed: 37, spin: [0, 0], skew: [0, 0] },
];

const SPARKLES = [
  { x: 430, y: 230, r: 18, color: "#FFFFFF" },
  { x: 1500, y: 250, r: 20, color: "#FFD166" },
  { x: 610, y: 120, r: 14, color: "#FFD166" },
  { x: 1300, y: 130, r: 16, color: "#FFFFFF" },
  { x: 200, y: 330, r: 16, color: "#FFFFFF" },
  { x: 1720, y: 300, r: 18, color: "#FFD166" },
  { x: 420, y: 520, r: 26, color: "#FFFFFF" },
  { x: 1510, y: 500, r: 30, color: "#FFD166" },
  { x: 300, y: 780, r: 20, color: "#FFD166" },
  { x: 1640, y: 760, r: 24, color: "#FFFFFF" },
  { x: 560, y: 960, r: 14, color: "#FFFFFF" },
  { x: 1360, y: 970, r: 16, color: "#FFD166" },
  { x: 780, y: 150, r: 12, color: "#FFFFFF" },
  { x: 1160, y: 110, r: 14, color: "#FFD166" },
];
const NONE = SPARKLES.map(() => 0);
const SPARKLE_SIZE = SPARKLES.map((s) => s.r * 0.7);
const POP_DELAY = SPARKLES.map((_, i) => (((i * 5) % SPARKLES.length) / SPARKLES.length) * 0.7);
const TWINKLE_DELAY = SPARKLES.map((_, i) => (((i * 9) % SPARKLES.length) / SPARKLES.length) * 0.8);

function Title() {
  const { seconds } = useCurrentTime();
  return (
    <Text
      split="char"
      className="absolute"
      style={{
        left: 0,
        top: 575,
        width: 1920,
        textAlign: "center",
        fontSize: 210,
        fontWeight: 900,
        letterSpacing: -3,
        lineHeight: 1,
        color: INK,
        textShadow: `0px 14px 0px ${SHADE}`,
      }}
      perUnit={(unit) => {
        const t = seconds - 0.15 - unit.index * 0.06;
        const pop = interpolate(t, [0, 0.9], [0, 1], { easing: OUT_ELASTIC });
        const rise = spring(t, { duration: 0.55, bounce: 0.45 });
        const tilt = interpolate(t, [0, 0.6], [1, 0], { easing: OUT_BACK });
        const bob = Math.sin((seconds - unit.index * 0.11) * 3.4) * 7 * clamp(t - 0.8, 0, 1);
        return {
          translate: `0px ${(1 - rise) * 170 + bob}px`,
          scale: pop,
          rotate: `${(unit.index % 2 === 0 ? -16 : 16) * tilt}deg`,
        };
      }}
    >
      Congrats!
    </Text>
  );
}

function Cannon({ side }: { side: number }) {
  const { seconds } = useCurrentTime();
  return (
    <>
      {FAMILIES.map((family) =>
        particles(seconds - BURST, {
          seed: family.seed + (side > 0 ? 0 : 100),
          count: PIECES,
          emitter: { x: side > 0 ? -80 : 1910, y: 1010, width: 90, height: 90 },
          birth: { interval: 0.004 },
          lifetime: 4.6,
          velocity: { x: side > 0 ? [500, 2100] : [-2100, -500], y: [-2900, -1600] },
          gravity: { x: 0, y: 900 },
          forces: [curlNoise({ seed: family.seed, scale: 0.005, strength: 650 }), drag(2.2)],
        }).map((p) => {
          const [w, h] = family.shapes[p.index];
          const turn = family.spin[0] + (family.spin[1] - family.spin[0]) * p.progress;
          const lean = family.skew[0] + (family.skew[1] - family.skew[0]) * p.progress;
          return (
            <View
              key={`${family.id}-${p.key}`}
              className="absolute"
              style={{
                left: family.round ? p.x - w / 2 : p.x,
                top: family.round ? p.y - h / 2 : p.y,
                width: w,
                height: h,
                borderRadius: family.round ? w / 2 : 0,
                backgroundColor: PIECE_COLORS[p.index],
                opacity: p.alive ? 1 : 0,
                transform: `rotate(${turn}deg) skewX(${lean}deg)`,
              }}
            />
          );
        }),
      )}
    </>
  );
}

function Medal() {
  const { seconds } = useCurrentTime();
  const t = seconds - MEDAL_IN;
  return (
    // The pivot sits far above the canvas: the ribbon hangs from it and the disc swings below.
    // The drop is a translation, so motionBlur "auto" smears it; the spin squashes the disc,
    // which only a Shutter can blur.
    <View
      key="hang"
      className="absolute"
      style={{
        left: MEDAL_CENTER.x,
        top: MEDAL_CENTER.y - 780,
        width: 0,
        height: 0,
        translate: `0px ${MEDAL.at(t).drop}px`,
        motionBlur: "auto",
        rotate: `${MEDAL.at(t).swing + wiggle(seconds, { seed: 4, frequency: 0.35, amplitude: 0.8 })}deg`,
      }}
    >
      <svg
        key="ribbon"
        className="absolute"
        width={1}
        height={1}
        style={{ left: 0, top: 0, overflow: "visible" }}
      >
        <path d="M 76 0 L 160 0 L 40 644 L -44 644 Z" fill="#118AB2" />
        <path d="M -160 0 L -76 0 L 44 644 L -40 644 Z" fill="#EF476F" />
      </svg>
      <View
        key="clasp"
        className="absolute"
        style={{
          left: -54,
          top: 598,
          width: 108,
          height: 50,
          borderRadius: 16,
          border: "3px solid #FFE9A6",
          backgroundImage: "linear-gradient(180deg, #FFD978, #E39A2F)",
        }}
      />
      <View
        key="disc"
        className="absolute"
        style={{ left: -150, top: 630, width: 300, height: 300 }}
      >
        <Shutter samples={4} angle={180}>
          <MedalFace />
        </Shutter>
      </View>
    </View>
  );
}

function MedalFace() {
  const { seconds } = useCurrentTime();
  const face = Math.cos(MEDAL.at(seconds - MEDAL_IN).spin);
  const shine = interpolate(seconds, [3.9, 4.7], [-160, 460], { easing: IN_OUT_CUBIC });
  return (
    // A coin turning about its vertical axis: squash it by the cosine of the turn.
    <View
      key="turn"
      className="absolute overflow-hidden rounded-full"
      style={{ left: 0, top: 0, width: 300, height: 300, scale: `${face} 1` }}
    >
      <View
        key="rim"
        className="absolute rounded-full"
        style={{
          left: 0,
          top: 0,
          width: 300,
          height: 300,
          backgroundImage: "radial-gradient(circle at 34% 30%, #FFE7A0, #F6B73C 58%, #D98E1F)",
        }}
      />
      <View
        key="face"
        className="absolute rounded-full"
        style={{
          left: 24,
          top: 24,
          width: 252,
          height: 252,
          border: "6px solid #FFF0C4",
          backgroundImage: "radial-gradient(circle at 40% 34%, #FFEDB5, #FFD166 52%, #F3AE3F)",
        }}
      />
      <View
        key="ring"
        className="absolute rounded-full"
        style={{ left: 50, top: 50, width: 200, height: 200, border: "5px dotted #FFF3CF" }}
      />
      <svg
        key="emblem"
        className="absolute"
        width={1}
        height={1}
        style={{ left: 150, top: 156, overflow: "visible" }}
      >
        <path d={EMBLEM} fill="#FFF7DD" />
      </svg>
      <View
        key="shine"
        className="absolute"
        style={{
          left: 0,
          top: -60,
          width: 70,
          height: 420,
          rotate: "24deg",
          translate: `${shine}px 0px`,
          backgroundImage: "linear-gradient(90deg, #FFFFFF00, #FFFFFF8C, #FFFFFF00)",
        }}
      />
      <View
        key="shade"
        className="absolute rounded-full"
        style={{
          left: 0,
          top: 0,
          width: 300,
          height: 300,
          backgroundColor: interpolate(Math.abs(face), [0, 1], ["#5A2A0078", "#5A2A0008"]),
        }}
      />
    </View>
  );
}

function ShootingStar() {
  const { seconds } = useCurrentTime();
  const flight = interpolate(seconds, STAR_FLIGHT, [0, 1], { easing: IN_SINE });
  if (!(flight > 0 && flight < 1)) return null;
  return (
    <View
      key="comet"
      className="absolute"
      style={{
        left: 0,
        top: 0,
        width: 0,
        height: 0,
        offsetPath: `path('${STAR_ARC}')`,
        offsetDistance: `${100 * flight}%`,
        offsetRotate: "auto",
        motionBlur: "auto",
      }}
    >
      <svg
        key="tail"
        className="absolute"
        width={1}
        height={1}
        style={{ left: 0, top: 0, overflow: "visible" }}
      >
        <defs>
          <linearGradient id="tail" gradientUnits="userSpaceOnUse" x1={-380} y1={0} x2={0} y2={0}>
            <stop offset={0} stopColor="#FFD16600" />
            <stop offset={0.7} stopColor="#FFD16699" />
            <stop offset={1} stopColor="#FFFFFFF2" />
          </linearGradient>
        </defs>
        <path d={TAIL} fill="url(#tail)" />
      </svg>
      <View
        key="head"
        className="absolute"
        style={{
          left: -48,
          top: -48,
          width: 96,
          height: 96,
          borderRadius: 48,
          backgroundImage:
            "radial-gradient(circle closest-side, #FFF4C8E6, #FFD16666 45%, #FFD16600)",
        }}
      >
        <svg
          key="star"
          className="absolute"
          width={1}
          height={1}
          style={{ left: 48, top: 48, overflow: "visible", rotate: `${seconds * 540}deg` }}
        >
          <path d={STAR} fill="#FFFBEA" />
        </svg>
      </View>
    </View>
  );
}

export default function Celebrate() {
  const { seconds, progress } = useCurrentTime();
  const burst = interpolate(seconds, [0.1, 1.4], [0, 1], { easing: OUT_EXPO });
  const pop = interpolate(seconds, [1.3, 2.8], [0, 1], { easing: OUT_CUBIC });
  const subtitle = interpolate(seconds, [2.2, 2.75], [0, 1], { easing: OUT_CUBIC });
  // Each sparkle grows in after its own delay, twinkles out of step and turns a quarter.
  const sizes = field({ from: NONE, to: SPARKLE_SIZE, progress: pop, stagger: POP_DELAY });
  const glints = field({
    from: NONE.map(() => 0.3),
    to: NONE.map(() => 1),
    progress: 0.5 + 0.5 * Math.sin(seconds * 4.4),
    stagger: TWINKLE_DELAY,
  });
  return (
    <View
      className="relative h-full w-full"
      style={{
        backgroundImage:
          "radial-gradient(circle at 50% 36%, #8475F2 0%, #6C5CE7 48%, #5847D8 100%)",
      }}
    >
      <svg
        key="rays"
        className="absolute"
        width={1}
        height={1}
        style={{
          left: MEDAL_CENTER.x,
          top: MEDAL_CENTER.y,
          overflow: "visible",
          rotate: `${seconds * 7}deg`,
          scale: burst,
        }}
      >
        <path d={RAYS} fill="#FFFFFF12" />
      </svg>

      <svg key="sparkles" className="absolute inset-0" width={1920} height={1080}>
        {SPARKLES.map((sparkle, i) => (
          <path
            key={`sparkle-${i}`}
            d={SPARKLE}
            fill={sparkle.color}
            fillOpacity={glints[i]}
            transform={`translate(${sparkle.x} ${sparkle.y}) rotate(${90 * progress}) scale(${sizes[i]})`}
          />
        ))}
      </svg>

      <View key="sky" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 560 }}>
        <Echo count={6} delay={0.008} decay={0.6}>
          <ShootingStar />
        </Echo>
      </View>

      <Medal key="medal" />

      <Title key="title" />

      <Cannon key="left-cannon" side={1} />
      <Cannon key="right-cannon" side={-1} />

      <View
        key="subtitle"
        className="absolute flex items-center justify-center"
        style={{
          left: 0,
          top: 838,
          width: 1920,
          height: 80,
          opacity: subtitle,
          translate: `0px ${(1 - subtitle) * 28}px`,
        }}
      >
        <View
          key="pill"
          className="flex items-center"
          style={{
            height: 76,
            paddingLeft: 40,
            paddingRight: 40,
            borderRadius: 38,
            backgroundColor: SHADE,
            border: "2px solid #FFFFFF40",
          }}
        >
          <Text key="label" style={{ fontSize: 42, fontWeight: 700, letterSpacing: 1, color: INK }}>
            1,000,000 views · thank you!
          </Text>
        </View>
      </View>
    </View>
  );
}
