// Bloom: nine hundred seeds on one spiral. The angle between one seed and the next drifts toward
// the golden angle, and the arms of the spiral rearrange until the seeds pack evenly.
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

const NIGHT = "#0B0A0D";
const CREAM = "#F4EEE4";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const CENTER = { x: 1180, y: 540 };
const RADIUS = 452;
const COUNT = 900;
const GOLDEN = 137.50776;

// Each seed's distance, size and colour are fixed; only its angle and its swell follow the time.
const SEEDS = Array.from({ length: COUNT }, (_, i) => {
  const u = (i + 0.5) / COUNT;
  return {
    key: `seed-${i}`,
    index: i,
    distance: RADIUS * Math.sqrt(u),
    size: 3 + 5 * Math.sqrt(u),
    delay: 0.1 + 1.1 * u,
    color: interpolate(u, [0, 0.3, 0.62, 1], ["#FFE9A8", "#FF9F45", "#F2416B", "#6B3BFF"], {
      colorSpace: "oklch",
    }),
  };
});

export default function Bloom() {
  const { seconds } = useCurrentTime();
  const angle = interpolate(seconds, [0.4, 4.1], [GOLDEN - 0.62, GOLDEN], {
    easing: "easeInOutCubic",
  });
  const turn = seconds * 5;
  const meta = interpolate(seconds, [0.6, 1.4], [0, 1], { easing: "easeOutCubic" });
  const title = interpolate(seconds, [0.9, 1.9], [0, 1], { easing: "easeOutCubic" });
  const settled = interpolate(seconds, [4.0, 4.5], [0, 1], { easing: "easeOutCubic" });
  const ring = interpolate(seconds, [0.3, 1.8], [0, 1], { easing: "easeInOutCubic" });
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: NIGHT }}>
      <View
        key="glow"
        className="absolute inset-0"
        style={{
          backgroundImage: `radial-gradient(circle 620px at ${CENTER.x}px ${CENTER.y}px, #3A1D5C66 0%, #0B0A0D00 100%)`,
        }}
      />

      <svg
        key="field"
        className="absolute inset-0"
        width={1920}
        height={1080}
        style={{ filter: "bloom(26px 0.55 0.45)" }}
      >
        <circle
          cx={CENTER.x}
          cy={CENTER.y}
          r={RADIUS + 34}
          fill="none"
          stroke="#F4EEE433"
          strokeWidth={1.5}
          pathLength={1}
          strokeDasharray="1 1"
          strokeDashoffset={1 - ring}
          transform={`rotate(-90 ${CENTER.x} ${CENTER.y})`}
        />
        <g transform={`translate(${CENTER.x} ${CENTER.y}) rotate(${turn})`}>
          {SEEDS.map((seed) => {
            const a = (seed.index * angle * Math.PI) / 180;
            const born = interpolate(seconds, [seed.delay, seed.delay + 0.7], [0, 1], {
              easing: "easeOutCubic",
            });
            // A swell that runs outward from the middle.
            const swell = 0.8 + 0.2 * Math.sin(seconds * 2.6 - seed.distance * 0.016);
            return (
              <circle
                key={seed.key}
                cx={seed.distance * Math.cos(a)}
                cy={seed.distance * Math.sin(a)}
                r={seed.size * born * swell}
                fill={seed.color}
              />
            );
          })}
        </g>
      </svg>

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: CREAM }}>N° 04 — BLOOM</Text>
        <Text style={{ ...LABEL, color: CREAM }}>900 SEEDS</Text>
      </View>

      <View
        key="title"
        className="absolute flex flex-col"
        style={{
          left: 96,
          top: 318,
          width: 560,
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
            color: CREAM,
          }}
        >
          One angle,
        </Text>
        <Text
          style={{
            fontFamily: "Instrument",
            fontStyle: "italic",
            fontSize: 148,
            lineHeight: 0.95,
            letterSpacing: -3,
            color: "#FF9F45",
          }}
        >
          repeated.
        </Text>
        <View style={{ marginTop: 56, width: 420, height: 1.5, backgroundColor: "#F4EEE44D" }} />
        <Text
          style={{
            marginTop: 28,
            fontFamily: "GeistMono",
            fontSize: 20,
            letterSpacing: 4,
            color: "#F4EEE499",
          }}
        >
          ANGLE BETWEEN SEEDS
        </Text>
        <View className="flex items-baseline" style={{ marginTop: 10, gap: 20 }}>
          <Text
            style={{
              fontFamily: "GeistMono",
              fontSize: 64,
              fontWeight: 300,
              letterSpacing: -1,
              color: CREAM,
              fontVariantNumeric: "tabular-nums",
            }}
          >
            {formatNumber(angle, { decimals: 3 })}°
          </Text>
          <Text
            style={{
              fontFamily: "GeistMono",
              fontSize: 20,
              letterSpacing: 4,
              color: "#FF9F45",
              opacity: settled,
            }}
          >
            GOLDEN
          </Text>
        </View>
      </View>
    </View>
  );
}
