// Kinetic title: "MAKE IT MOVE" springs in letter by letter over sweeping colour blocks, then the
// key word fills from outline to ink while the variable font widens it.
// Shows Text split/perUnit, rangeSelector, spring presets, easeOutBack and cubic-bezier(), a
// two-colour Span, fontVariationSettings, WebkitTextStroke, defineSequence/stage and props; a
// short entrance blur, one glow and one chromatic-aberration hit are its only filters.
// Render: valle motion render examples/01-kinetic-title.motion.tsx -o 01-kinetic-title.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

export const controls = {
  props: {
    word: string({ default: "MOVE" }),
    tagline: string({ default: "kinetic type in a single file" }),
    accent: color({ default: "#FF5E5B" }),
  },
};

const INK = "#1B1B3A";
const PAPER = "#FFD23F";
const TEAL = "#00CECB";
const INDIGO = "#5B5FEF";
const PINK = "#FF2E93";

// Brisk start, soft landing: the stroke of a marker pen.
const SWEEP = "cubic-bezier(0.3, 0.1, 0.25, 1)";

const SHOT = defineSequence({
  wipe: stage({ duration: 0.66 }),
  blocks: stage({ at: 0.3, duration: 0.8 }),
  letters: stage({ at: 0.36, duration: 0.6 }),
  word: stage({ at: 0.72, duration: 0.55 }),
  fill: stage({ after: "word", overlap: 0.05, duration: 1.1 }),
  hit: stage({ after: "fill", overlap: 0.1, duration: 0.35 }),
  tagline: stage({ after: "fill", overlap: 0.15, duration: 0.6 }),
  wave: stage({ after: "tagline", delay: 0.3, duration: 1.3 }),
});

// A four-point sparkle of the given size, drawn from its top-left corner.
const sparkle = (size) => {
  const m = size / 2;
  const a = size * 0.38;
  const b = size * 0.62;
  return path(`M ${m} 0 C ${m * 1.1} ${a} ${b} ${m * 0.9} ${size} ${m} C ${b} ${m * 1.1} ${m * 1.1} ${b} ${m} ${size} C ${m * 0.9} ${b} ${a} ${m * 1.1} 0 ${m} C ${a} ${m * 0.9} ${m * 0.9} ${a} ${m} 0 Z`);
};
const SPARKLE = sparkle(100);
const SPARK = sparkle(26);

// Three full-screen panels wipe off to the right, the top one first.
const PANELS = [
  { key: "wipe-indigo", color: INDIGO, order: 2 },
  { key: "wipe-teal", color: TEAL, order: 1 },
  { key: "wipe-coral", color: "#FF5E5B", order: 0 },
];
// The marquee along the bottom: sparkles and dots, scrolling left.
const SPARKS = Array.from({ length: 16 }, (_, index) => ({ key: `spark-${index}`, x: 30 + index * 168 }));
const DOTS = Array.from({ length: 16 }, (_, index) => ({
  key: `dot-${index}`, x: 127 + index * 168, color: [TEAL, "#FF5E5B", PINK][index % 3],
}));

// A colour block behind a line of type; it sweeps open from the left.
function Block({ color, progress, tilt, inset, radius }) {
  return (
    <View className="absolute" style={{ inset, borderRadius: radius, backgroundColor: color,
      rotate: tilt, scale: point(progress, 1), transformOrigin: "0% 50%" }} />
  );
}

// A small shape that pops in on a spring, then bobs and sways.
function Sticker({ x, y, size, delay, sway, children }) {
  const { seconds } = useCurrentTime();
  const pop = spring(seconds - delay, { preset: "bouncy" });
  return (
    <View className="absolute" style={{ left: x - size / 2, top: y - size / 2, width: size,
      height: size, translate: point(0, sin(seconds * 1.6 + delay * 5) * 12),
      rotate: `${sway * sin(seconds * 1.3 + delay * 3)}deg`, scale: 0.4 + 0.6 * pop,
      opacity: clamp(pop * 3, 0, 1) }}>
      {children}
    </View>
  );
}

// A disc of halftone dots: one tiled gradient, turning slowly.
function Halftone({ x, y, size, dot, delay }) {
  const { seconds } = useCurrentTime();
  const grow = interpolate(seconds, [delay, delay + 0.9], [0, 1], { easing: "easeOutCubic" });
  return (
    <View className="absolute" style={{ left: x - size / 2, top: y - size / 2, width: size,
      height: size, borderRadius: size / 2,
      backgroundImage: `radial-gradient(circle, ${dot} 30%, transparent 34%)`,
      backgroundSize: "26px 26px", scale: grow, rotate: `${8 * seconds}deg` }} />
  );
}

function Label({ text, delay }) {
  const { seconds } = useCurrentTime();
  const slide = interpolate(seconds, [delay, delay + 0.6], [0, 1], { easing: "easeOutBack" });
  return (
    <Text style={{ fontSize: 24, fontWeight: 800, letterSpacing: 6, color: INK,
      opacity: clamp(slide, 0, 1), translate: point(0, (1 - slide) * -28) }}>{text}</Text>
  );
}

function FirstLine() {
  const { seconds } = useCurrentTime();
  const sweep = interpolate(stageProgress(seconds, SHOT.blocks), [0, 1], [0, 1], { easing: SWEEP });
  return (
    <View className="relative flex" style={{ paddingInline: 40 }}>
      <Block color={TEAL} progress={sweep} tilt="-2deg" inset="40% 0px 4% 0px" radius={6} />
      <Text split="char" style={{ fontSize: 210, fontWeight: 900, lineHeight: 1, letterSpacing: 6,
        color: INK }}
        perUnit={(unit) => {
          const pop = spring(seconds - SHOT.letters.start - unit.index * 0.05, { preset: "wobbly" });
          const size = 0.3 + 0.7 * pop;
          return { opacity: clamp(pop * 2.5, 0, 1), scale: point(size, size),
            translate: point(0, (1 - pop) * -120), rotate: (1 - pop) * 18 };
        }}>
        {"MAKE "}<Span style={{ color: INDIGO }}>IT</Span>
      </Text>
    </View>
  );
}

function KeyWord({ word, accent }) {
  const { seconds } = useCurrentTime();
  const sweep = interpolate(seconds, [0.42, 1.22], [0, 1], { easing: SWEEP });
  const enter = interpolate(stageProgress(seconds, SHOT.word), [0, 1], [0, 1],
    { easing: "easeOutExpo" });
  const fill = interpolate(stageProgress(seconds, SHOT.fill), [0, 1], [0, 1],
    { easing: "easeInOutCubic" });
  const hit = stageProgress(seconds, SHOT.hit);
  const thump = interpolate(hit, [0, 0.3, 1], [1, 1.05, 1],
    { easing: ["easeOutCubic", "easeInOutCubic"] });
  const wave = stageProgress(seconds, SHOT.wave);
  // Blur only while the word comes into focus, aberration only during the hit.
  const focusing = seconds < SHOT.word.start + SHOT.word.duration;
  const hitting = hit > 0 && hit < 1;
  return (
    <View className="relative flex" style={{ paddingInline: 48, marginTop: -6 }}>
      <Block color={accent} progress={sweep} tilt="1.5deg" inset="15% 0px 8% 0px" radius={8} />
      <Text split="char" style={{ fontSize: 400, lineHeight: 1, color: INK,
        fontVariationSettings: `"wght" ${300 + 600 * fill}, "wdth" ${62.5 + 37.5 * fill}`,
        WebkitTextStroke: `6px ${INK}`, scale: (1.3 - 0.3 * enter) * thump,
        transformOrigin: "50% 25%",
        opacity: clamp(enter * 2, 0, 1),
        filter: focusing ? `blur(${4 * (1 - enter)}px)`
          : hitting ? `chromatic-aberration(${18 * (1 - hit)}px)` : "none" }}
        perUnit={(unit) => {
          // Each letter fills as the selector's edge crosses it and gives a small kick.
          const inked = rangeSelector(unit, { start: 0, end: fill, softness: 0.05, shape: "smooth" });
          const kick = rangeSelector(unit, { start: 0, end: 0.2, offset: fill - 0.1, softness: 0.2,
            shape: "triangle" });
          const lift = rangeSelector(unit, { start: 0, end: 0.3, offset: 1.6 * wave - 0.55,
            softness: 0.3, shape: "triangle" });
          return {
            color: interpolate(inked, [0, 1], ["#1B1B3A00", INK]),
            scale: point(1 + 0.12 * kick, 1 + 0.12 * kick),
            translate: point(0, -40 * lift),
          };
        }}>
        {word}
      </Text>
    </View>
  );
}

function Tagline({ text }) {
  const { seconds } = useCurrentTime();
  const open = interpolate(stageProgress(seconds, SHOT.tagline), [0, 1], [0, 1], { easing: SWEEP });
  // The words follow the pill and land once it has opened.
  const rise = spring(seconds - SHOT.tagline.start - 0.35, { preset: "gentle" });
  return (
    <View className="relative flex items-center" style={{ marginTop: 36, paddingBlock: 16,
      paddingInline: 34, gap: 24 }}>
      <Block color={INDIGO} progress={open} tilt="-1deg" inset="0px" radius={48} />
      <Text style={{ fontSize: 46, fontWeight: 900, letterSpacing: 3, color: "#FFE3F1",
        filter: `glow(14px 1.3 ${PINK})`, scale: rise }}>TSX</Text>
      <Text style={{ fontSize: 46, fontWeight: 600, letterSpacing: 1, color: "#FFFFFF",
        opacity: clamp(rise, 0, 1), translate: point((1 - rise) * 48, 0) }}>{text}</Text>
    </View>
  );
}

export default function KineticTitle(props) {
  const { seconds } = useCurrentTime();
  const band = interpolate(seconds, [0.6, 1.3], [0, 1], { easing: "easeOutCubic" });
  return (
    <Scene className="relative h-full w-full flex flex-col items-center justify-center"
      style={{ backgroundColor: PAPER, paddingBottom: 64 }}>
      <Halftone x={260} y={250} size={360} dot="#FF5E5B66" delay={0.5} />
      <Halftone x={1700} y={760} size={300} dot="#5B5FEF55" delay={0.65} />
      <View className="absolute flex justify-between" style={{ left: 72, right: 72, top: 56 }}>
        <Label key="brand" text="VALLE / MOTION" delay={0.5} />
        <Label key="issue" text="01 — KINETIC TITLE" delay={0.6} />
      </View>
      <Sticker x={240} y={520} size={130} delay={1.3} sway={0}>
        <Circle className="absolute inset-0" cx={65} cy={65} r={46} fill="none" stroke={TEAL}
          strokeWidth={16} />
      </Sticker>
      <Sticker x={1680} y={300} size={100} delay={1.45} sway={14}>
        <Path className="absolute inset-0" d={SPARKLE} fill={INDIGO} />
      </Sticker>
      <Sticker x={300} y={850} size={150} delay={1.6} sway={8}>
        <Polyline className="absolute inset-0"
          points={[point(10, 95), point(42, 55), point(74, 95), point(106, 55), point(138, 95)]}
          fill="none" stroke={PINK} strokeWidth={14} strokeLinecap="round" strokeLinejoin="round" />
      </Sticker>
      <Sticker x={1660} y={560} size={70} delay={1.75} sway={0}>
        <Circle className="absolute inset-0" cx={35} cy={35} r={30} fill={props.accent} />
      </Sticker>
      <FirstLine />
      <KeyWord word={props.word} accent={props.accent} />
      <Tagline text={props.tagline} />
      <View className="absolute overflow-hidden" style={{ left: 0, right: 0, bottom: 0, height: 64,
        backgroundColor: INK, translate: point(0, (1 - band) * 64) }}>
        {SPARKS.map((spark) => (
          <Path key={spark.key} d={SPARK} fill={PAPER}
            style={{ translate: point(spark.x - 80 * seconds, 19) }} />
        ))}
        {DOTS.map((dot) => (
          <Circle key={dot.key} cx={dot.x - 80 * seconds} cy={32} r={8} fill={dot.color} />
        ))}
      </View>
      {PANELS.map((panel) => (
        <View key={panel.key} className="absolute" style={{ left: -400, top: 0, width: 2720,
          height: 1080, backgroundColor: panel.color, transform: "skewX(-14deg)",
          translate: point(2600 * interpolate(staggerProgress(seconds, SHOT.wipe, panel.order, 0.09),
            [0, 1], [0, 1], { easing: "easeInOutSine" }), 0) }} />
      ))}
    </Scene>
  );
}
