// Ridge: sixty-four white lines on black, each one row of the same noise. Where the noise is
// high a line lifts and hides the lines behind it, and the rows read as a range of hills that
// never stops moving. One SVG path a line.
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
    GeistMono: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geistmono/GeistMono%5Bwght%5D.ttf",
    }),
  },
};

const BLACK = "#050505";
const WHITE = "#F5F3EE";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const COUNT = 64;
const STEPS = 120;
const CENTER = 1180;
const TOP = 250;
const PITCH = 10.4;

const LINES = Array.from({ length: COUNT }, (_, i) => ({
  key: `line-${i}`,
  index: i,
  y: TOP + i * PITCH,
  // The plane recedes: the lines at the back are shorter.
  half: 430 + 190 * (i / (COUNT - 1)),
}));
const POINTS = Array.from({ length: STEPS + 1 }, (_, i) => i / STEPS);

// Where the hills stand along a line: two peaks, and flat ground at both ends.
function range(u) {
  return Math.exp(-((u - 0.4) ** 2) / 0.016) + 0.72 * Math.exp(-((u - 0.63) ** 2) / 0.009);
}

export default function Ridge() {
  const { seconds } = useCurrentTime();
  const meta = interpolate(seconds, [0.5, 1.3], [0, 1], { easing: "easeOutCubic" });
  const title = interpolate(seconds, [1.0, 2.0], [0, 1], { easing: "easeOutCubic" });
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: BLACK }}>
      <svg key="lines" className="absolute inset-0" width={1920} height={1080}>
        {LINES.map((line) => {
          // The lines arrive flat, back to front, and the hills rise out of them.
          const shown = interpolate(
            seconds,
            [0.1 + line.index * 0.012, 0.5 + line.index * 0.012],
            [0, 1],
          );
          const rise = interpolate(
            seconds,
            [0.5 + line.index * 0.014, 2.3 + line.index * 0.014],
            [0, 1],
            { easing: "easeOutExpo" },
          );
          let d = "";
          for (const u of POINTS) {
            const n = noise2d(7, u * 11 + line.index * 0.29, line.index * 0.16 - seconds * 0.55);
            const ripple = noise2d(3, u * 40, line.index * 0.9 + seconds * 0.8) - 0.5;
            const y = line.y - rise * range(u) * (20 + 250 * n * n) - 5 * ripple;
            d += `${u === 0 ? "M" : "L"}${CENTER - line.half + 2 * line.half * u} ${y}`;
          }
          return (
            <path
              key={line.key}
              d={d}
              fill={BLACK}
              stroke={WHITE}
              strokeWidth={1.8}
              strokeLinejoin="round"
              opacity={shown}
            />
          );
        })}
      </svg>
      <View
        key="grain"
        className="absolute inset-0"
        style={{ filter: `film-grain(${Math.floor(seconds * 24)} 0.1 1.5px)` }}
      />

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: WHITE }}>N° 07 — RIDGE</Text>
        <Text style={{ ...LABEL, color: WHITE }}>64 LINES · ONE NOISE</Text>
      </View>

      <View
        key="title"
        className="absolute flex flex-col"
        style={{
          left: 96,
          top: 356,
          opacity: title,
          translate: `0px ${(1 - title) * 24}px`,
        }}
      >
        <Text
          style={{
            fontFamily: "Instrument",
            fontSize: 148,
            lineHeight: 0.95,
            letterSpacing: -3,
            color: WHITE,
          }}
        >
          Hills, from
        </Text>
        <Text
          style={{
            fontFamily: "Instrument",
            fontStyle: "italic",
            fontSize: 148,
            lineHeight: 0.95,
            letterSpacing: -3,
            color: WHITE,
          }}
        >
          noise.
        </Text>
        <View style={{ marginTop: 56, width: 420, height: 1.5, backgroundColor: "#F5F3EE4D" }} />
        <Text style={{ ...LABEL, marginTop: 28, color: "#F5F3EE99" }}>ROW OF THE NOISE</Text>
        <Text
          style={{
            marginTop: 10,
            fontFamily: "GeistMono",
            fontSize: 64,
            fontWeight: 300,
            letterSpacing: -1,
            color: WHITE,
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {formatNumber(seconds * 0.55, { decimals: 3 })}
        </Text>
      </View>

      <View
        key="foot"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, bottom: 64, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: WHITE }}>EACH LINE HIDES THE ONES BEHIND</Text>
        <Text style={{ ...LABEL, color: WHITE }}>SVG · 64 PATHS</Text>
      </View>
    </View>
  );
}
