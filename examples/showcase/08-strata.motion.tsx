// Strata: a year of visits as five streams stacked about one centre line. A reading line
// crosses the year and the total counts with it; then the stack comes apart, and each stream
// stands on its own. Every edge is one function, sampled a hundred times.
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

const PAPER = "#F1ECE2";
const INK = "#16140F";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const PLOT = { left: 96, width: 1728, middle: 600 };
const STEPS = 100;
const POINTS = Array.from({ length: STEPS + 1 }, (_, i) => i / STEPS);
const STREAMS = [
  { key: "search", name: "SEARCH", color: "#16140F", seed: 11, share: 1.0 },
  { key: "direct", name: "DIRECT", color: "#2340F5", seed: 23, share: 0.8 },
  { key: "social", name: "SOCIAL", color: "#7A3BFF", seed: 37, share: 0.7 },
  { key: "mail", name: "EMAIL", color: "#F03A17", seed: 41, share: 0.55 },
  { key: "other", name: "REFERRAL", color: "#FF9F45", seed: 53, share: 0.45 },
];
const QUARTERS = ["JAN", "APR", "JUL", "OCT", "DEC"].map((name, i) => ({ name, u: i / 4 }));
// A visit is this many pixels of a stream's height.
const VISITS = 31;

// How thick a stream is at `u` of the year: it grows through the year. The data does not
// change with the time, so the total a day reads is the same on every frame.
function thickness(stream, u) {
  const grown = 0.42 + 0.58 * u ** 1.2;
  const n = noise2d(stream.seed, u * 7.5, 0.5);
  return stream.share * grown * (16 + 330 * n * n);
}

export default function Strata() {
  const { seconds } = useCurrentTime();
  const meta = interpolate(seconds, [0.2, 1.0], [0, 1], { easing: "easeOutCubic" });
  // The reading line crosses the year.
  const read = interpolate(seconds, [0.4, 3.1], [0, 1], { easing: "easeInOutCubic" });
  // Then the stack comes apart.
  const apart = interpolate(seconds, [3.4, 4.3], [0, 1], { easing: "easeInOutCubic" });
  const gap = 34 * apart;
  const title = interpolate(seconds, [3.5, 4.3], [0, 1], { easing: "easeOutCubic" });

  // Each stream's two edges, from the top of the stack down.
  const whole = (u) => STREAMS.reduce((sum, s) => sum + thickness(s, u), 0);
  const paths = STREAMS.map((stream, index) => {
    let upper = "";
    let lower = "";
    for (const u of POINTS) {
      // The centre line wanders, so the stack reads as a current, not a wedge.
      const middle = PLOT.middle + 70 * (noise2d(5, u * 2.6, 0.5) - 0.5);
      let top = middle - (whole(u) + gap * (STREAMS.length - 1)) / 2;
      for (let k = 0; k < index; k++) top += thickness(STREAMS[k], u) + gap;
      const x = PLOT.left + PLOT.width * u;
      upper += `${u === 0 ? "M" : "L"}${x} ${top}`;
      lower = `L${x} ${top + thickness(stream, u)}` + lower;
    }
    return { stream, d: `${upper}${lower}Z` };
  });
  const x = PLOT.left + PLOT.width * read;
  const total = whole(read);

  return (
    <View className="relative h-full w-full" style={{ backgroundColor: PAPER }}>
      <svg key="plot" className="absolute inset-0" width={1920} height={1080}>
        {QUARTERS.map((quarter) => (
          <line
            key={quarter.name}
            x1={PLOT.left + PLOT.width * quarter.u}
            y1={300}
            x2={PLOT.left + PLOT.width * quarter.u}
            y2={900}
            stroke="#16140F1F"
            strokeWidth={1.5}
          />
        ))}
      </svg>
      <View
        key="streams"
        className="absolute inset-0"
        style={{ clipPath: `inset(0px ${1920 - x}px 0px 0px)` }}
      >
        <svg className="absolute inset-0" width={1920} height={1080}>
          {paths.map((path) => (
            <path key={path.stream.key} d={path.d} fill={path.stream.color} />
          ))}
        </svg>
      </View>
      <svg key="reader" className="absolute inset-0" width={1920} height={1080}>
        <line x1={x} y1={300} x2={x} y2={900} stroke={INK} strokeWidth={2} opacity={1 - apart} />
        <circle cx={x} cy={300} r={7} fill={INK} opacity={1 - apart} />
      </svg>

      {QUARTERS.map((quarter) => (
        <Text
          key={quarter.name}
          className="absolute"
          style={{
            ...LABEL,
            left: PLOT.left + PLOT.width * quarter.u - 60,
            top: 916,
            width: 120,
            textAlign: quarter.u === 0 ? "left" : quarter.u === 1 ? "right" : "center",
            paddingLeft: quarter.u === 0 ? 60 : 0,
            paddingRight: quarter.u === 1 ? 60 : 0,
            color: "#16140F99",
            opacity: meta,
          }}
        >
          {quarter.name}
        </Text>
      ))}

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: INK }}>N° 08 — STRATA</Text>
        <View className="flex" style={{ gap: 36 }}>
          {STREAMS.map((stream) => (
            <View key={stream.key} className="flex items-center" style={{ gap: 12 }}>
              <View
                style={{ width: 14, height: 14, borderRadius: 7, backgroundColor: stream.color }}
              />
              <Text style={{ ...LABEL, color: INK }}>{stream.name}</Text>
            </View>
          ))}
        </View>
      </View>

      <View
        key="total"
        className="absolute flex items-baseline"
        style={{ left: 96, top: 128, gap: 28, opacity: meta }}
      >
        <Text
          style={{
            fontFamily: "Instrument",
            fontSize: 150,
            lineHeight: 1,
            letterSpacing: -4,
            color: INK,
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {formatNumber(total * VISITS, { decimals: 0, grouping: true })}
        </Text>
        <Text style={{ ...LABEL, color: "#16140F99" }}>
          VISITS A DAY · DAY {padNumber(1 + 364 * read, { width: 3 })}
        </Text>
      </View>

      <View
        key="foot"
        className="absolute flex items-end justify-between"
        style={{ left: 96, right: 96, bottom: 56 }}
      >
        <Text
          style={{
            fontFamily: "Instrument",
            fontSize: 76,
            lineHeight: 0.95,
            letterSpacing: -2,
            color: INK,
            opacity: title,
            translate: `0px ${(1 - title) * 18}px`,
          }}
        >
          A year, <Span style={{ fontStyle: "italic", color: "#F03A17" }}>in layers.</Span>
        </Text>
        <Text style={{ ...LABEL, color: INK, opacity: meta }}>5 STREAMS · 365 DAYS</Text>
      </View>
    </View>
  );
}
