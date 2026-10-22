// Vertical story: the progress bars fill in turn, the avatar pops and a quote is revealed line by
// line through a gradient mask, over a background whose hues drift.
// Shows useComposition() for a responsive layout, linear/radial/conic gradients in oklch, per-frame
// gradient paint, Mask and MaskSource, mixBlendMode and isolation, measureText, repeatProgress and
// interpolate in oklch.
// Render: valle motion render examples/05-story-vertical.motion.tsx -o 05-story-vertical.mp4

export const composition = { width: 1080, height: 1920, fps: 60, duration: 6 };

const INK = "#3A0CA3";
const HEADLINE = "Start small.";
const QUOTE = ["Great motion is", "a thousand tiny", "decisions, made", "with care."];
const QUOTE_SIZE = 64;
const QUOTE_LINE = 86;
const COVER_HEIGHT = 330;
// The reply bar along the bottom, and the like button in it.
const REPLY_BOTTOM = 64;
const REPLY_HEIGHT = 96;
const ICON_GAP = 28;

const SHOT = defineSequence({
  bars: stage({ at: 0.2, duration: 4.9 }),
  avatar: stage({ at: 0.15, duration: 0.6 }),
  card: stage({ at: 0.35, duration: 0.9 }),
  headline: stage({ at: 0.85, duration: 0.7 }),
  quote: stage({ at: 1.3, duration: 0.8 }),
  marker: stage({ at: 3.1, duration: 0.55 }),
  author: stage({ after: "marker", overlap: 0.2, duration: 0.6 }),
  like: stage({ at: 3.7, duration: 0.8 }),
  hint: stage({ at: 3.6, duration: 1.5 }),
});

// A 16-point sunburst badge, 260 px across.
const BURST = path(
  Array.from({ length: 32 }, (_, index) => {
    const angle = (index / 32) * Math.PI * 2;
    const radius = index % 2 === 0 ? 130 : 112;
    const x = 130 + radius * Math.cos(angle);
    const y = 130 + radius * Math.sin(angle);
    return `${index === 0 ? "M" : "L"} ${x.toFixed(2)} ${y.toFixed(2)}`;
  }).join(" ") + " Z",
);
// Hearts that float up after the like.
const FLOATERS = Array.from({ length: 7 }, (_, index) => ({
  key: `float-${index}`,
  delay: 0.1 + index * 0.06,
  drift: [-40, 30, -12, 52, -56, 22, -26][index],
  rise: 200 + (index % 3) * 40,
  size: 0.6 + (index % 4) * 0.1,
  phase: index * 1.7,
  white: index % 2 === 1,
}));
const HEART = path(
  "M 32 56 C 12 42 0 30 0 17 C 0 7 8 0 17 0 C 24 0 29 4 32 9 C 35 4 40 0 47 0 C 56 0 64 7 64 17 C 64 30 52 42 32 56 Z",
);

function StoryBars({ width, margin }) {
  const { seconds } = useCurrentTime();
  const run = stageProgress(seconds, SHOT.bars) * 3;
  const bar = (width - 2 * margin - 2 * 10) / 3;
  return (
    <View className="absolute flex" style={{ left: margin, top: 34, gap: 10 }}>
      {[0, 1, 2].map((index) => (
        <View key={`bar-${index}`} style={{ width: bar, height: 8, borderRadius: 4,
          backgroundColor: "#FFFFFF59", overflow: "hidden" }}>
          <View key={`fill-${index}`} style={{ width: bar, height: 8, backgroundColor: "#FFFFFF",
            scale: point(clamp(run - index, 0, 1), 1), transformOrigin: "0% 50%" }} />
        </View>
      ))}
    </View>
  );
}

function Avatar() {
  const { seconds } = useCurrentTime();
  const pop = spring(seconds - SHOT.avatar.start, { preset: "bouncy" });
  return (
    <View className="relative" style={{ width: 112, height: 112, scale: 0.5 + 0.5 * pop,
      opacity: clamp(pop * 3, 0, 1) }}>
      <View className="absolute inset-0" style={{ borderRadius: 56, rotate: `${120 * seconds}deg`,
        background: "conic-gradient(from 90deg in oklch, #FEDA75, #FA7E1E, #D62976, #962FBF, #4F5BD5, #FEDA75)" }} />
      <View className="absolute flex items-center justify-center" style={{ inset: 6, borderRadius: 50,
        backgroundColor: "#FFFFFF" }}>
        <View className="flex items-center justify-center" style={{ width: 88, height: 88,
          borderRadius: 44, background: "linear-gradient(135deg in oklch, #7B2FF7, #F107A3)" }}>
          <Text style={{ fontSize: 34, fontWeight: 800, letterSpacing: 1, color: "#FFFFFF" }}>VS</Text>
        </View>
      </View>
    </View>
  );
}

function Header({ margin }) {
  const { seconds } = useCurrentTime();
  const slide = interpolate(seconds, [0.3, 0.9], [0, 1], { easing: "easeOutCubic" });
  return (
    <View className="absolute flex items-center" style={{ left: margin, right: margin, top: 74,
      gap: 24 }}>
      <Avatar />
      <View className="flex flex-col" style={{ gap: 4, flexGrow: 1, opacity: slide,
        translate: point((1 - slide) * -30, 0) }}>
        <Text style={{ fontSize: 36, fontWeight: 800, color: "#FFFFFF" }}>valle.studio</Text>
        <Text style={{ fontSize: 28, fontWeight: 500, color: "#FFFFFFCC" }}>2h · Motion notes</Text>
      </View>
      <Text style={{ fontSize: 44, fontWeight: 800, letterSpacing: 2, color: "#FFFFFF",
        opacity: slide }}>···</Text>
    </View>
  );
}

// The cover art: two soft colour blobs drifting over a gradient, all in one background. The yellow
// blob fades out through pink, so its rim warms the violet instead of greying it.
function Cover({ inner }) {
  const { seconds } = useCurrentTime();
  const turn = seconds * 0.9;
  return (
    <View className="relative flex items-end" style={{ width: inner, height: COVER_HEIGHT,
      borderRadius: 36, overflow: "hidden", padding: 28,
      backgroundImage: `radial-gradient(circle at ${30 + 16 * cos(turn)}% ${36 + 18 * sin(turn)}% in oklch, #FFD166 0%, #F7258500 44%), radial-gradient(circle at ${72 + 14 * sin(turn * 0.8)}% ${60 + 16 * cos(turn * 0.8)}% in oklch, #F72585 0%, #F7258500 50%), linear-gradient(135deg in oklch, #7209B7, #4361EE)` }}>
      <View className="flex items-center" style={{ paddingBlock: 10, paddingInline: 22,
        borderRadius: 30, backgroundColor: "#FFFFFF33", gap: 12 }}>
        <View style={{ width: 14, height: 14, borderRadius: 7, backgroundColor: "#FFD166" }} />
        <Text style={{ fontSize: 26, fontWeight: 700, letterSpacing: 3, color: "#FFFFFF" }}>
          STORY 02 / 03
        </Text>
      </View>
    </View>
  );
}

function Headline({ inner, size }) {
  const { seconds } = useCurrentTime();
  const reveal = interpolate(stageProgress(seconds, SHOT.headline), [0, 1], [0, 1],
    { easing: "easeOutCubic" });
  const height = Math.ceil(size * 1.1);
  // The mask's gradient slides down every frame: opaque above, clear below.
  return (
    <Mask rect={rect(0, 0, inner, height)} style={{ width: inner, height }}
      paint={linearGradient(point(0, height * (2.2 * reveal - 1.2)),
        point(0, height * (2.2 * reveal - 0.2)),
        [gradientStop(0, "#FFFFFFFF"), gradientStop(1, "#FFFFFF00")])}>
      <Text style={{ fontSize: size, fontWeight: 900, lineHeight: 1.1, whiteSpace: "nowrap",
        color: INK, translate: point(0, (1 - reveal) * 36) }}>{HEADLINE}</Text>
    </Mask>
  );
}

function Quote({ inner, marker }) {
  const { seconds } = useCurrentTime();
  const height = QUOTE_LINE * QUOTE.length;
  const swipe = interpolate(stageProgress(seconds, SHOT.marker), [0, 1], [0, 1],
    { easing: "easeInOutCubic" });
  return (
    <View className="relative" style={{ width: inner, height }}>
      <Mask rect={rect(0, 0, inner, height)} className="absolute" style={{ left: 0, top: 0,
        width: inner, height }}>
        <Text style={{ width: inner, fontSize: QUOTE_SIZE, fontWeight: 650,
          lineHeight: `${QUOTE_LINE}px`, whiteSpace: "pre-line", color: INK }}>
          {QUOTE.join("\n")}
        </Text>
        {/* One soft-edged bar per line; each slides right and uncovers its line. */}
        <MaskSource>
          {QUOTE.map((line, index) => (
            <View key={`wipe-${index}`} className="absolute" style={{ left: 0,
              top: index * QUOTE_LINE, width: inner * 2, height: QUOTE_LINE,
              backgroundImage: "linear-gradient(90deg, #FFFFFF 50%, #FFFFFF00 62%)",
              translate: point(-1.24 * inner * (1 - interpolate(
                staggerProgress(seconds, SHOT.quote, index, 0.32), [0, 1], [0, 1],
                { easing: "easeOutCubic" })), 0) }} />
          ))}
        </MaskSource>
      </Mask>
      {/* A highlighter stroke: multiply keeps the ink dark and tints only the paper. */}
      <View className="absolute" style={{ left: -10, top: 3 * QUOTE_LINE + 30, width: marker + 20,
        height: 46, borderRadius: 10, backgroundColor: "#FFD60A", mixBlendMode: "multiply",
        rotate: "-1.5deg", scale: point(swipe, 1), transformOrigin: "0% 50%" }} />
    </View>
  );
}

function Card({ inner, pad, headlineSize, marker }) {
  const { seconds } = useCurrentTime();
  const rise = interpolate(stageProgress(seconds, SHOT.card), [0, 1], [0, 1],
    { easing: "easeOutCubic" });
  const author = interpolate(stageProgress(seconds, SHOT.author), [0, 1], [0, 1],
    { easing: "easeOutCubic" });
  return (
    <View className="flex flex-col" style={{ padding: pad, gap: 30, borderRadius: 56,
      backgroundColor: "#FFFFFFE6", isolation: "isolate", translate: point(0, (1 - rise) * 40),
      scale: 0.94 + 0.06 * rise, rotate: "-1deg", opacity: clamp(rise * 1.6, 0, 1) }}>
      <Cover inner={inner} />
      <Text style={{ marginTop: 6, fontSize: 26, fontWeight: 800, letterSpacing: 6,
        color: "#3A0CA399" }}>THOUGHT OF THE DAY</Text>
      <Headline inner={inner} size={headlineSize} />
      <Quote inner={inner} marker={marker} />
      <Text style={{ fontSize: 30, fontWeight: 600, color: "#3A0CA3B3", opacity: author,
        translate: point((1 - author) * 30, 0) }}>— valle.studio, 2026</Text>
    </View>
  );
}

// A printed sticker: multiply lets the card and the cover show through its ink.
function Sticker({ x, y }) {
  const { seconds } = useCurrentTime();
  const pop = spring(seconds - 1.0, { preset: "wobbly" });
  return (
    <View className="absolute flex items-center justify-center" style={{ left: x - 130,
      top: y - 130, width: 260, height: 260, mixBlendMode: "multiply",
      rotate: `${-10 + 5 * sin(seconds * 1.4)}deg`, scale: 0.4 + 0.6 * pop,
      opacity: clamp(pop * 3, 0, 1) }}>
      <Path className="absolute inset-0" d={BURST} fill="#FFD166"
        style={{ rotate: `${20 * seconds}deg` }} />
      <Text style={{ fontSize: 46, fontWeight: 900, lineHeight: 1, textAlign: "center",
        color: INK, whiteSpace: "pre-line" }}>{"DAY\n06"}</Text>
    </View>
  );
}

function ReplyBar({ margin }) {
  const { seconds } = useCurrentTime();
  const enter = interpolate(seconds, [0.6, 1.2], [0, 1], { easing: "easeOutCubic" });
  const like = spring(seconds - SHOT.like.start, { preset: "bouncy" });
  const ring = stageProgress(seconds, SHOT.like);
  return (
    <View className="absolute flex items-center" style={{ left: margin, right: margin,
      bottom: REPLY_BOTTOM, gap: ICON_GAP, opacity: enter, translate: point(0, (1 - enter) * 60) }}>
      <View className="flex items-center" style={{ flexGrow: 1, height: REPLY_HEIGHT, borderRadius: 48,
        border: "3px solid #FFFFFFB3", paddingInline: 40 }}>
        <Text style={{ fontSize: 32, fontWeight: 500, color: "#FFFFFFE6" }}>Send message</Text>
      </View>
      <View className="relative" style={{ width: 64, height: 56 }}>
        <Circle cx={32} cy={28} r={20 + 34 * ring} fill="none" strokeWidth={4}
          stroke={interpolate(ring, [0, 1], ["#FFFFFFCC", "#FFFFFF00"])} />
        <Path className="absolute inset-0" d={HEART} fill="none" stroke="#FFFFFF" strokeWidth={5} />
        <Path className="absolute inset-0" d={HEART} fill="#F72585"
          style={{ scale: like, transformOrigin: "50% 50%" }} />
      </View>
      <Polygon points={[point(0, 4), point(60, 28), point(0, 52), point(10, 28)]} fill="none"
        stroke="#FFFFFF" strokeWidth={5} strokeLinejoin="round" style={{ width: 60, height: 56 }} />
    </View>
  );
}

function MoreHint() {
  const { seconds } = useCurrentTime();
  const show = interpolate(seconds, [SHOT.hint.start, SHOT.hint.start + 0.4], [0, 1]);
  const hop = repeatProgress(seconds, SHOT.hint, 3);
  return (
    <View className="absolute flex flex-col items-center" style={{ left: 0, right: 0, bottom: 196,
      gap: 6, opacity: show, translate: point(0, -18 * sin(hop * TAU / 2)) }}>
      <Polyline points={[point(4, 22), point(26, 4), point(48, 22)]} fill="none" stroke="#FFFFFF"
        strokeWidth={6} strokeLinecap="round" strokeLinejoin="round" style={{ width: 52, height: 26 }} />
      <Text style={{ fontSize: 24, fontWeight: 800, letterSpacing: 6, color: "#FFFFFF" }}>MORE</Text>
    </View>
  );
}

export default function StoryVertical() {
  const { width, height } = useComposition();
  const { seconds, progress } = useCurrentTime();
  const margin = Math.round(width * 0.056);
  const pad = Math.round(width * 0.052);
  const inner = width - 2 * margin - 2 * pad;
  // Fit the headline to the card: measure once at 100 px, then scale.
  const measured = measureText(HEADLINE, { fontSize: 100, fontWeight: 900 });
  const headlineSize = Math.floor((inner * 0.97 * 100) / measured.width);
  const marker = measureText(QUOTE[3], { fontSize: QUOTE_SIZE, fontWeight: 650 }).width;
  const top = interpolate(progress, [0, 1], ["#FF5F6D", "#F72585"], { colorSpace: "oklch" });
  const bottom = interpolate(progress, [0, 1], ["#FFC371", "#FF9A76"], { colorSpace: "oklch" });
  const drift = seconds * 0.7;
  const fan = interpolate(stageProgress(seconds, SHOT.card), [0, 1], [0, 1],
    { easing: "easeOutCubic" });
  // Hearts float up from the like button, the third item of the reply bar. One shared path batch
  // scales them from point(0, 0), so each is offset by its size to stay centred.
  const heartX = width - margin - 60 - ICON_GAP - 32;
  const heartY = height - REPLY_BOTTOM - REPLY_HEIGHT / 2;
  const liked = seconds - SHOT.like.start;
  return (
    <Scene className="relative h-full w-full flex flex-col justify-center"
      style={{ paddingTop: 300, paddingBottom: 250, paddingInline: margin,
        backgroundImage: `radial-gradient(circle at ${24 + 10 * sin(drift)}% ${18 + 6 * cos(drift)}% in oklch, #FFFFFF66 0%, #FFFFFF00 34%), radial-gradient(circle at ${80 + 8 * cos(drift)}% ${86 + 5 * sin(drift)}% in oklch, #FFFFFF4D 0%, #FFFFFF00 30%), linear-gradient(165deg in oklch, ${top}, ${bottom})` }}>
      <View className="absolute" style={{ left: margin + 20, right: margin + 20, top: 470,
        bottom: 330, borderRadius: 56, rotate: `${4 * fan}deg`,
        backgroundColor: interpolate(fan, [0, 1], ["#FFFFFF00", "#FFFFFF40"]) }} />
      <Card inner={inner} pad={pad} headlineSize={headlineSize} marker={marker} />
      <Sticker x={width - margin - 120} y={330} />
      <StoryBars width={width} margin={margin} />
      <Header margin={margin} />
      <MoreHint />
      <ReplyBar margin={margin} />
      <View className="absolute" style={{ left: heartX - 32, top: heartY - 28, width: 64,
        height: 56 }}>
        {FLOATERS.map((heart) => (
          <Path key={heart.key} d={HEART}
            fill={heart.white
              ? interpolate(liked - heart.delay, [0, 0.15, 0.7, 1.2],
                ["#FFFFFF00", "#FFFFFF", "#FFFFFF", "#FFFFFF00"])
              : interpolate(liked - heart.delay, [0, 0.15, 0.7, 1.2],
                ["#FF4D8D00", "#FF4D8D", "#FF4D8D", "#FF4D8D00"])}
            style={{ scale: heart.size, transformOrigin: point(0, 0),
              translate: point(32 * (1 - heart.size) + 10 * sin(5 * (liked - heart.delay) + heart.phase)
                + heart.drift * clamp((liked - heart.delay) / 1.2, 0, 1),
                28 * (1 - heart.size) - heart.rise * interpolate(liked - heart.delay, [0, 1.2], [0, 1],
                  { easing: "easeOutQuad" })) }} />
        ))}
      </View>
    </Scene>
  );
}
