// Celebrate: a bouncing title, confetti cannons, a medal on its ribbon and a shooting star.
// Shows particles() with curlNoise and drag, GeometryBatch side fields and field(), simulate(),
// easeOutElastic, springs, wiggle, motionPath, and three bounded effects: a 4-sample Shutter on
// the medal, an Echo trail and motionBlur "auto" on the star.
// Render: valle motion render examples/08-celebrate.motion.tsx -o 08-celebrate.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

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
  return path(`${corners.join(" ")} Z`);
}

// 18 thin wedges around (0, 0), long enough to reach past every corner of the canvas.
const RAYS = path(
  Array.from({ length: 18 }, (_, i) => {
    const a = (i / 18) * Math.PI * 2;
    const b = a + Math.PI / 18;
    const r = 1900;
    return `M 0 0 L ${(r * Math.cos(a)).toFixed(1)} ${(r * Math.sin(a)).toFixed(1)} L ${(r * Math.cos(b)).toFixed(1)} ${(r * Math.sin(b)).toFixed(1)} Z`;
  }).join(" "),
);
const EMBLEM = roundCorners(starPath(70, 30), 6);
const STAR = roundCorners(starPath(30, 13), 3);
const TAIL = path("M 2 -12 Q -120 -5 -380 0 Q -120 5 2 12 Z");
const STAR_ARC = path("M -260 120 Q 900 10 2180 300");
const SPARKLE = path("M 0 -1 Q 0.12 -0.12 1 0 Q 0.12 0.12 0 1 Q -0.12 0.12 -1 0 Q -0.12 -0.12 0 -1 Z");

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
      drop: s.drop + dropV * dt, dropV,
      spin: s.spin + spinV * dt, spinV,
      swing: s.swing + swingV * dt, swingV,
    };
  },
});

// Confetti: three spin families per cannon. Arrays as long as `count` give each piece its own
// color and shape; two-entry arrays run from birth to the end of its life.
const PIECES = 72;
const PIECE_COLORS = Array.from({ length: PIECES }, (_, i) => CONFETTI[(i * 3) % CONFETTI.length]);
const STRIPS = Array.from({ length: PIECES }, (_, i) => (i % 3 === 0 ? point(17, 17) : point(30, 12)));
const DOTS = Array.from({ length: PIECES }, (_, i) => (i % 2 === 0 ? point(15, 15) : point(11, 11)));
const FAMILIES = [
  { id: "strips", geometry: "rect", shapes: STRIPS, seed: 11, spin: [0, 2100], skew: [-50, 40] },
  { id: "flips", geometry: "rect", shapes: STRIPS, seed: 23, spin: [140, -1700], skew: [45, -55] },
  { id: "dots", geometry: "circle", shapes: DOTS, seed: 37, spin: [0, 0], skew: [0, 0] },
];

const SPARKLES = [
  { x: 430, y: 230, r: 18, color: "#FFFFFF" }, { x: 1500, y: 250, r: 20, color: "#FFD166" },
  { x: 610, y: 120, r: 14, color: "#FFD166" }, { x: 1300, y: 130, r: 16, color: "#FFFFFF" },
  { x: 200, y: 330, r: 16, color: "#FFFFFF" }, { x: 1720, y: 300, r: 18, color: "#FFD166" },
  { x: 420, y: 520, r: 26, color: "#FFFFFF" }, { x: 1510, y: 500, r: 30, color: "#FFD166" },
  { x: 300, y: 780, r: 20, color: "#FFD166" }, { x: 1640, y: 760, r: 24, color: "#FFFFFF" },
  { x: 560, y: 960, r: 14, color: "#FFFFFF" }, { x: 1360, y: 970, r: 16, color: "#FFD166" },
  { x: 780, y: 150, r: 12, color: "#FFFFFF" }, { x: 1160, y: 110, r: 14, color: "#FFD166" },
];
const SPARKLE_AT = SPARKLES.map((s) => point(s.x, s.y));
const SPARKLE_SIZE = SPARKLES.map((s) => point(s.r * 1.4, s.r * 1.4));
const SPARKLE_FILL = SPARKLES.map((s) => s.color);
const POP_DELAY = SPARKLES.map((_, i) => ((i * 5) % SPARKLES.length) / SPARKLES.length * 0.7);
const TWINKLE_DELAY = SPARKLES.map((_, i) => ((i * 9) % SPARKLES.length) / SPARKLES.length * 0.8);

function Title() {
  const { seconds } = useCurrentTime();
  return (
    <Text split="char" className="absolute"
      style={{ left: 0, top: 575, width: 1920, textAlign: "center", fontSize: 210,
        fontWeight: 900, letterSpacing: -3, lineHeight: 1, color: INK,
        textShadow: `0px 14px 0px ${SHADE}` }}
      perUnit={(unit) => {
        const t = seconds - 0.15 - unit.index * 0.06;
        const pop = interpolate(t, [0, 0.9], [0, 1], { easing: "easeOutElastic" });
        const rise = spring(t, { duration: 0.55, bounce: 0.45 });
        const tilt = interpolate(t, [0, 0.6], [1, 0], { easing: "easeOutBack" });
        const bob = Math.sin((seconds - unit.index * 0.11) * 3.4) * 7 * clamp(t - 0.8, 0, 1);
        return {
          translate: point(0, (1 - rise) * 170 + bob),
          scale: point(pop, pop),
          rotate: (unit.index % 2 === 0 ? -16 : 16) * tilt,
        };
      }}>
      Congrats!
    </Text>
  );
}

function Cannon({ side }: { side: number }) {
  const { seconds } = useCurrentTime();
  return (
    <>
      {FAMILIES.map((family) => (
        <GeometryBatch key={family.id} geometry={family.geometry}
          positions={particles(seconds - BURST, {
            seed: family.seed + (side > 0 ? 0 : 100), count: PIECES,
            emitter: side > 0 ? rect(-80, 1010, 90, 90) : rect(1910, 1010, 90, 90),
            birth: { interval: 0.004 }, lifetime: 4.6,
            velocity: { x: side > 0 ? [500, 2100] : [-2100, -500], y: [-2900, -1600] },
            gravity: point(0, 900),
            forces: [curlNoise({ seed: family.seed, scale: 0.005, strength: 650 }), drag(2.2)],
          })}
          sizes={family.shapes} fills={PIECE_COLORS} rotations={family.spin} skewXs={family.skew}
          style={{ position: "absolute", left: 0, top: 0, width: 1920, height: 1080 }} />
      ))}
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
    <View key="hang" className="absolute"
      style={{ left: MEDAL_CENTER.x, top: MEDAL_CENTER.y - 780, width: 0, height: 0,
        translate: point(0, MEDAL.at(t).drop), motionBlur: "auto",
        rotate: `${MEDAL.at(t).swing + wiggle(seconds, { seed: 4, frequency: 0.35, amplitude: 0.8 })}deg` }}>
      <Path key="ribbon-back" d={path("M 76 0 L 160 0 L 40 644 L -44 644 Z")} fill="#118AB2"
        style={{ position: "absolute", left: 0, top: 0 }} />
      <Path key="ribbon-front" d={path("M -160 0 L -76 0 L 44 644 L -40 644 Z")} fill="#EF476F"
        style={{ position: "absolute", left: 0, top: 0 }} />
      <View key="clasp" className="absolute"
        style={{ left: -54, top: 598, width: 108, height: 50, borderRadius: 16,
          border: "3px solid #FFE9A6", backgroundImage: "linear-gradient(180deg, #FFD978, #E39A2F)" }} />
      <View key="disc" className="absolute" style={{ left: -150, top: 630, width: 300, height: 300 }}>
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
  const shine = interpolate(seconds, [3.9, 4.7], [-160, 460], { easing: "easeInOutCubic" });
  return (
    // A coin turning about its vertical axis: squash it by the cosine of the turn.
    <View key="turn" className="absolute overflow-hidden rounded-full"
      style={{ left: 0, top: 0, width: 300, height: 300, scale: point(face, 1) }}>
      <View key="rim" className="absolute rounded-full"
        style={{ left: 0, top: 0, width: 300, height: 300,
          backgroundImage: "radial-gradient(circle at 34% 30%, #FFE7A0, #F6B73C 58%, #D98E1F)" }} />
      <View key="face" className="absolute rounded-full"
        style={{ left: 24, top: 24, width: 252, height: 252, border: "6px solid #FFF0C4",
          backgroundImage: "radial-gradient(circle at 40% 34%, #FFEDB5, #FFD166 52%, #F3AE3F)" }} />
      <View key="ring" className="absolute rounded-full"
        style={{ left: 50, top: 50, width: 200, height: 200, border: "5px dotted #FFF3CF" }} />
      <Path key="emblem" d={EMBLEM} fill="#FFF7DD" style={{ position: "absolute", left: 150, top: 156 }} />
      <View key="shine" className="absolute"
        style={{ left: 0, top: -60, width: 70, height: 420, rotate: "24deg",
          translate: point(shine, 0),
          backgroundImage: "linear-gradient(90deg, #FFFFFF00, #FFFFFF8C, #FFFFFF00)" }} />
      <View key="shade" className="absolute rounded-full"
        style={{ left: 0, top: 0, width: 300, height: 300,
          backgroundColor: interpolate(Math.abs(face), [0, 1], ["#5A2A0078", "#5A2A0008"]) }} />
    </View>
  );
}

function ShootingStar() {
  const { seconds } = useCurrentTime();
  const flight = interpolate(seconds, STAR_FLIGHT, [0, 1], { easing: "easeInSine" });
  return (
    <View key="comet" className="absolute"
      visible={flight > 0 && flight < 1}
      style={{ width: 0, height: 0, motionPath: motionPath(STAR_ARC, flight, { rotate: "auto" }),
        motionBlur: "auto" }}>
      <Path key="tail" d={TAIL} style={{ position: "absolute", left: 0, top: 0 }}
        fill={linearGradient(point(-380, 0), point(0, 0),
          [gradientStop(0, "#FFD16600"), gradientStop(0.7, "#FFD16699"), gradientStop(1, "#FFFFFFF2")])} />
      <View key="head" className="absolute"
        style={{ left: -48, top: -48, width: 96, height: 96 }}>
        <Circle key="halo" cx={48} cy={48} r={46}
          fill={radialGradient(point(48, 48), 46,
            [gradientStop(0, "#FFF4C8E6"), gradientStop(0.45, "#FFD16666"), gradientStop(1, "#FFD16600")])} />
        <Path key="star" d={STAR} fill="#FFFBEA"
          style={{ position: "absolute", left: 48, top: 48, rotate: `${seconds * 540}deg` }} />
      </View>
    </View>
  );
}

export default function Celebrate() {
  const { seconds, progress } = useCurrentTime();
  const burst = interpolate(seconds, [0.1, 1.4], [0, 1], { easing: "easeOutExpo" });
  const pop = interpolate(seconds, [1.3, 2.8], [0, 1], { easing: "easeOutCubic" });
  const subtitle = interpolate(seconds, [2.2, 2.75], [0, 1], { easing: "easeOutCubic" });
  return (
    <Scene className="relative h-full w-full"
      style={{ backgroundImage: "radial-gradient(circle at 50% 36%, #8475F2 0%, #6C5CE7 48%, #5847D8 100%)" }}>
      <Path key="rays" d={RAYS} fill="#FFFFFF12"
        style={{ position: "absolute", left: MEDAL_CENTER.x, top: MEDAL_CENTER.y,
          rotate: `${seconds * 7}deg`, scale: burst }} />

      {/* Fixed batch: field() eases sizes in with a per-sparkle delay; opacities fold into the
          fill alpha, so the twinkle costs no layer. */}
      <GeometryBatch key="sparkles" geometry={SPARKLE} positions={SPARKLE_AT} fills={SPARKLE_FILL}
        sizes={field({ from: 0, to: SPARKLE_SIZE, progress: pop, stagger: POP_DELAY })}
        opacities={field({ from: 0.3, to: 1, progress: 0.5 + 0.5 * Math.sin(seconds * 4.4),
          stagger: TWINKLE_DELAY })}
        rotations={field({ from: 0, to: 90, progress })}
        style={{ position: "absolute", left: 0, top: 0, width: 1920, height: 1080 }} />

      <View key="sky" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 560 }}>
        <Echo count={6} delay={0.008} decay={0.6}>
          <ShootingStar />
        </Echo>
      </View>

      <Medal key="medal" />

      <Title key="title" />

      <Cannon key="left-cannon" side={1} />
      <Cannon key="right-cannon" side={-1} />

      <View key="subtitle" className="absolute flex items-center justify-center"
        style={{ left: 0, top: 838, width: 1920, height: 80, opacity: subtitle,
          translate: point(0, (1 - subtitle) * 28) }}>
        <View key="pill" className="flex items-center"
          style={{ height: 76, paddingLeft: 40, paddingRight: 40, borderRadius: 38,
            backgroundColor: SHADE, border: "2px solid #FFFFFF40" }}>
          <Text key="label" style={{ fontSize: 42, fontWeight: 700, letterSpacing: 1, color: INK }}>
            1,000,000 views · thank you!
          </Text>
        </View>
      </View>
    </Scene>
  );
}
