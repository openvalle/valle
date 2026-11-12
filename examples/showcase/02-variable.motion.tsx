// Variable: one word in a variable font, a wave of weight and width running through its
// letters and colouring them, until every letter locks at its heaviest and widest.
// Designed at 1920 x 1080, 30 fps, 5 s.

export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

// The fonts are Google Fonts' files through jsDelivr, at a pinned commit, under the SIL Open Font License 1.1.
export const controls = {
  resources: {
    Archivo: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/archivo/Archivo%5Bwdth,wght%5D.ttf",
    }),
    GeistMono: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geistmono/GeistMono%5Bwght%5D.ttf",
    }),
  },
};

const NIGHT = "#0B0A0D";
const CREAM = "#F4EEE4";
const ORANGE = "#FF9F45";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };

const WORD = "RHYTHM";
const LETTERS = WORD.split("").map((letter, index) => ({ key: `letter-${index}`, letter, index }));
// The curve under the word: the wave itself, drawn in dots, one large dot a letter.
const CURVE = { left: 96, width: 1728, top: 742, height: 104 };
const DOTS = Array.from({ length: 109 }, (_, i) => ({ key: `dot-${i}`, u: (i / 108) * 5 }));

// How heavy and wide the letter at `u` is, from 0 to 1: flat as it arrives, then the wave,
// then locked at 1.
function level(seconds, u) {
  const depth = interpolate(seconds, [0.6, 1.7], [0, 1], { easing: "easeInOutSine" });
  const lock = interpolate(seconds, [3.6, 4.25], [0, 1], { easing: "easeInOutCubic" });
  const wave = (0.5 + 0.5 * Math.sin(seconds * 2.4 - u * 0.72)) * depth;
  return wave + (1 - wave) * lock;
}

// Light and narrow is cool and dim; heavy and wide is warm.
function tint(amount) {
  return interpolate(amount, [0, 0.5, 1], ["#4B4F8C", CREAM, ORANGE], { colorSpace: "oklch" });
}

export default function Variable() {
  const { seconds } = useCurrentTime();
  const meta = interpolate(seconds, [0.9, 1.7], [0, 1], { easing: "easeOutCubic" });
  const lock = interpolate(seconds, [3.6, 4.25], [0, 1], { easing: "easeInOutCubic" });
  const locked = interpolate(seconds, [4.15, 4.5], [0, 1], { easing: "easeOutCubic" });
  const first = level(seconds, 0);
  const dot = spring(seconds - 1.4, { duration: 0.5, bounce: 0.4 });
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: NIGHT }}>
      <View
        key="glow"
        className="absolute inset-0"
        style={{
          backgroundImage:
            "radial-gradient(ellipse 980px 520px at 960px 430px, #3A1D5C80 0%, #0B0A0D00 100%)",
        }}
      />
      <View
        key="warmth"
        className="absolute inset-0"
        style={{
          opacity: lock,
          backgroundImage:
            "radial-gradient(ellipse 1100px 460px at 960px 430px, #FF7A2E3D 0%, #0B0A0D00 100%)",
        }}
      />

      <View
        key="word"
        className="absolute flex justify-center"
        style={{
          left: 0,
          top: 236,
          width: 1920,
          height: 400,
          filter: `bloom(44px ${0.1 + 0.12 * lock} 0.62)`,
        }}
      >
        <Text
          split="char"
          style={{
            fontFamily: "Archivo",
            fontSize: 288,
            lineHeight: "400px",
            whiteSpace: "pre",
            color: CREAM,
          }}
          perUnit={(unit) => {
            const amount = level(seconds, unit.index);
            const rise = interpolate(
              seconds,
              [0.1 + unit.index * 0.07, 1.0 + unit.index * 0.07],
              [0, 1],
              { easing: "easeOutExpo" },
            );
            return {
              fontVariationSettings: `"wght" ${100 + 800 * amount}, "wdth" ${62 + 63 * amount}`,
              color: tint(amount),
              opacity: rise,
              translate: `0px ${(1 - rise) * 150}px`,
            };
          }}
        >
          {WORD}
        </Text>
      </View>

      <svg
        key="curve"
        className="absolute inset-0"
        width={1920}
        height={1080}
        style={{ opacity: meta }}
      >
        <line
          x1={CURVE.left}
          y1={CURVE.top}
          x2={CURVE.left + CURVE.width}
          y2={CURVE.top}
          stroke="#F4EEE41F"
          strokeWidth={1.5}
        />
        <line
          x1={CURVE.left}
          y1={CURVE.top + CURVE.height}
          x2={CURVE.left + CURVE.width}
          y2={CURVE.top + CURVE.height}
          stroke="#F4EEE41F"
          strokeWidth={1.5}
        />
        {DOTS.map((point) => (
          <circle
            key={point.key}
            cx={CURVE.left + (point.u / 5) * CURVE.width}
            cy={CURVE.top + (1 - level(seconds, point.u)) * CURVE.height}
            r={2.4}
            fill="#F4EEE466"
          />
        ))}
        {LETTERS.map((item) => {
          const amount = level(seconds, item.index);
          const x = CURVE.left + (item.index / 5) * CURVE.width;
          const y = CURVE.top + (1 - amount) * CURVE.height;
          return (
            <g key={item.key}>
              <line
                x1={x}
                y1={y}
                x2={x}
                y2={CURVE.top + CURVE.height}
                stroke={tint(amount)}
                strokeWidth={1.5}
                opacity={0.5}
              />
              <circle cx={x} cy={y} r={9} fill={tint(amount)} />
            </g>
          );
        })}
      </svg>

      {LETTERS.map((item) => (
        <Text
          key={item.key}
          className="absolute"
          style={{
            ...LABEL,
            left: CURVE.left + (item.index / 5) * CURVE.width - 30,
            top: CURVE.top + CURVE.height + 22,
            width: 60,
            textAlign: "center",
            letterSpacing: 0,
            color: "#F4EEE499",
            opacity: meta,
          }}
        >
          {item.letter}
        </Text>
      ))}

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: CREAM }}>N° 02 — VARIABLE</Text>
        <Text style={{ ...LABEL, color: CREAM }}>ONE FONT FILE · TWO AXES</Text>
      </View>

      <View
        key="foot"
        className="absolute flex items-center justify-between"
        style={{ left: 96, right: 96, bottom: 64, opacity: meta }}
      >
        <View className="flex items-center" style={{ gap: 16 }}>
          <View
            style={{
              width: 16,
              height: 16,
              borderRadius: 8,
              backgroundColor: tint(first),
              scale: dot,
            }}
          />
          <Text style={{ ...LABEL, color: CREAM, fontVariantNumeric: "tabular-nums" }}>
            WGHT {padNumber(100 + 800 * first, { width: 3 })} · WDTH{" "}
            {padNumber(62 + 63 * first, { width: 3 })}
          </Text>
          <Text style={{ ...LABEL, color: ORANGE, opacity: locked }}>LOCKED</Text>
        </View>
        <Text style={{ ...LABEL, color: CREAM }}>ARCHIVO · 100—900 · 62—125</Text>
      </View>
    </View>
  );
}
