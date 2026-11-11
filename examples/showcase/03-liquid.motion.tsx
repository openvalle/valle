// Liquid: a word pools out of one soft mass, and drops drift through its letters, merging and
// parting. A luminance <Mask> whose source is blurred and then given hard contrast.
// Designed at 1920 x 1080, 30 fps, 5 s.

export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

// The fonts are Google Fonts' files through jsDelivr, at a pinned commit, under the SIL Open Font License 1.1.
export const controls = {
  resources: {
    Instrument: resource({
      kind: "font",
      default: [
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/instrumentserif/InstrumentSerif-Regular.ttf",
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/instrumentserif/InstrumentSerif-Italic.ttf",
      ],
    }),
    Geist: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geist/Geist%5Bwght%5D.ttf",
    }),
    GeistMono: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geistmono/GeistMono%5Bwght%5D.ttf",
    }),
  },
};

const BLUE = "#1B2CF2";
const CREAM = "#FFF4E6";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const CENTER = { x: 960, y: 500 };

// Drops on their own ellipses, all wider and taller than the word, so they graze its edges and
// never cover a letter: radius, the ellipse, turns a second and a phase.
const DROPS = [
  { r: 66, rx: 790, ry: 262, speed: 0.11, phase: 0.3 },
  { r: 46, rx: 700, ry: 236, speed: -0.16, phase: 1.9 },
  { r: 78, rx: 850, ry: 286, speed: 0.08, phase: 3.4 },
  { r: 34, rx: 680, ry: 222, speed: 0.21, phase: 4.6 },
  { r: 54, rx: 740, ry: 250, speed: -0.13, phase: 5.5 },
  { r: 28, rx: 690, ry: 214, speed: -0.24, phase: 2.6 },
  { r: 40, rx: 880, ry: 300, speed: 0.14, phase: 0.9 },
].map((drop, index) => ({ ...drop, key: `drop-${index}`, delay: 0.25 + index * 0.09 }));

// White on black, blurred and then pushed to hard contrast: shapes that come near each other join.
function Field() {
  const { seconds } = useCurrentTime();
  const blur = interpolate(seconds, [0, 1.9], [72, 11], { easing: "easeOutQuart" });
  const grow = interpolate(seconds, [0, 1.9], [0.82, 1], { easing: "easeOutQuart" });
  return (
    <View
      className="absolute"
      style={{
        left: 0,
        top: 0,
        width: 1920,
        height: 1080,
        backgroundColor: "#000000",
        filter: `blur(${blur}px) contrast(60)`,
      }}
    >
      <View
        key="word"
        className="absolute flex items-center justify-center"
        style={{ left: 0, top: CENTER.y - 320, width: 1920, height: 640, scale: grow }}
      >
        <Text
          style={{
            fontFamily: "Geist",
            fontWeight: 900,
            fontSize: 560,
            lineHeight: 1,
            letterSpacing: -22,
            color: "#FFFFFF",
          }}
        >
          melt
        </Text>
      </View>
      {DROPS.map((drop) => {
        const a = drop.phase + seconds * drop.speed * Math.PI * 2;
        const pop = spring(seconds - drop.delay, { duration: 0.9, bounce: 0.2 });
        return (
          <View
            key={drop.key}
            className="absolute"
            style={{
              left: CENTER.x - drop.r,
              top: CENTER.y - drop.r,
              width: drop.r * 2,
              height: drop.r * 2,
              borderRadius: drop.r,
              backgroundColor: "#FFFFFF",
              translate: `${drop.rx * Math.cos(a)}px ${drop.ry * Math.sin(a)}px`,
              scale: pop,
            }}
          />
        );
      })}
    </View>
  );
}

export default function Liquid() {
  const { seconds } = useCurrentTime();
  const meta = interpolate(seconds, [0.8, 1.6], [0, 1], { easing: "easeOutCubic" });
  const caption = interpolate(seconds, [1.7, 2.5], [0, 1], { easing: "easeOutCubic" });
  const blur = interpolate(seconds, [0, 1.9], [72, 11], { easing: "easeOutQuart" });
  const turn = 110 + 40 * Math.sin(seconds * 0.7);
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: BLUE }}>
      {/* A lighter pool behind the word, so the blue is not flat. */}
      <View
        key="pool"
        className="absolute inset-0"
        style={{
          backgroundImage:
            "radial-gradient(ellipse 70% 60% at 50% 46%, #3D55FF 0%, #1B2CF200 70%), linear-gradient(180deg, #1B2CF200 60%, #0E1AB0 100%)",
        }}
      />

      <Mask
        key="liquid"
        mode="luminance"
        className="absolute"
        style={{
          left: 0,
          top: 0,
          width: 1920,
          height: 1080,
          filter: "drop-shadow(0px 26px 34px #070F8A8C)",
        }}
        source={<Field />}
      >
        <View
          className="absolute inset-0"
          style={{
            backgroundImage: `linear-gradient(${turn}deg in oklch, #FFFFFF 18%, ${CREAM} 42%, #FFC2A1 68%, #FF7A4D 92%)`,
            filter: "film-grain(7 0.1 1.5px)",
          }}
        />
      </Mask>

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: CREAM }}>N° 03 — LIQUID</Text>
        <Text
          style={{
            fontFamily: "GeistMono",
            fontSize: 20,
            letterSpacing: 4,
            color: CREAM,
            fontVariantNumeric: "tabular-nums",
          }}
        >
          BLUR {formatNumber(blur, { decimals: 1 })} PX · CONTRAST 60
        </Text>
      </View>

      <View
        key="foot"
        className="absolute flex items-end justify-between"
        style={{
          left: 96,
          right: 96,
          bottom: 68,
          opacity: caption,
          translate: `0px ${(1 - caption) * 16}px`,
        }}
      >
        <Text
          style={{
            fontFamily: "Instrument",
            fontStyle: "italic",
            fontSize: 54,
            lineHeight: 1,
            color: CREAM,
          }}
        >
          Letters that pool, merge and part.
        </Text>
        <Text style={{ ...LABEL, color: CREAM }}>ONE MASK · SEVEN DROPS</Text>
      </View>
    </View>
  );
}
