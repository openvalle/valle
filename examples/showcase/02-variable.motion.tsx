// Variable: one word set five times in a variable font, a wave of weight and width running
// through its letters. Designed at 1920 x 1080, 30 fps, 5 s.

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

const PAPER = "#EEE9DF";
const INK = "#141312";
const RED = "#F03A17";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };

const WORD = "RHYTHM";
const ROWS = [0, 1, 2, 3, 4];
const ROW_HEIGHT = 172;
const TOP = 110;
const ACCENT_ROW = 2;

// One row: it rises out of its own window, then the wave passes through its letters.
function Row({ index }) {
  const { seconds } = useCurrentTime();
  const rise = interpolate(seconds, [0.15 + index * 0.08, 1.15 + index * 0.08], [0, 1], {
    easing: "easeOutExpo",
  });
  // The wave starts flat, so every row arrives as the same light, narrow word.
  const depth = interpolate(seconds, [0.7, 1.8], [0, 1], { easing: "easeInOutSine" });
  return (
    <View
      className="absolute flex justify-center overflow-hidden"
      style={{ left: 0, top: TOP + index * ROW_HEIGHT, width: 1920, height: ROW_HEIGHT }}
    >
      <Text
        split="char"
        style={{
          fontFamily: "Archivo",
          fontSize: 214,
          lineHeight: `${ROW_HEIGHT}px`,
          color: index === ACCENT_ROW ? RED : INK,
          whiteSpace: "pre",
          translate: `0px ${(1 - rise) * ROW_HEIGHT}px`,
        }}
        perUnit={(unit) => {
          const phase = seconds * 2.2 - unit.index * 0.62 - index * 0.85;
          const wave = (0.5 + 0.5 * Math.sin(phase)) * depth;
          return {
            fontVariationSettings: `"wght" ${120 + 780 * wave}, "wdth" ${62 + 63 * wave}`,
          };
        }}
      >
        {WORD}
      </Text>
    </View>
  );
}

export default function Variable() {
  const { seconds } = useCurrentTime();
  const meta = interpolate(seconds, [0.9, 1.6], [0, 1], { easing: "easeOutCubic" });
  const rules = interpolate(seconds, [0.1, 1.2], [0, 1], { easing: "easeOutExpo" });
  // The readout follows the first letter of the red row.
  const wave =
    (0.5 + 0.5 * Math.sin(seconds * 2.2 - ACCENT_ROW * 0.85)) *
    interpolate(seconds, [0.7, 1.8], [0, 1], { easing: "easeInOutSine" });
  const dot = spring(seconds - 1.5, { duration: 0.5, bounce: 0.4 });
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: PAPER }}>
      {ROWS.map((index) => (
        <View
          key={`rule-${index}`}
          className="absolute"
          style={{
            left: 96,
            top: TOP + index * ROW_HEIGHT,
            width: 1728,
            height: 1.5,
            backgroundColor: "#14131226",
            scale: `${rules} 1`,
            transformOrigin: index % 2 === 0 ? "0% 50%" : "100% 50%",
          }}
        />
      ))}
      <View
        key="rule-last"
        className="absolute"
        style={{
          left: 96,
          top: TOP + ROWS.length * ROW_HEIGHT,
          width: 1728,
          height: 1.5,
          backgroundColor: "#14131226",
          scale: `${rules} 1`,
          transformOrigin: "100% 50%",
        }}
      />

      {ROWS.map((index) => (
        <Row key={`row-${index}`} index={index} />
      ))}

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 56, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: INK }}>N° 02 — VARIABLE</Text>
        <Text style={{ ...LABEL, color: INK }}>ONE FONT FILE · TWO AXES</Text>
      </View>

      <View
        key="foot"
        className="absolute flex items-center justify-between"
        style={{ left: 96, right: 96, top: 1000, opacity: meta }}
      >
        <View className="flex items-center" style={{ gap: 16 }}>
          <View
            style={{ width: 16, height: 16, borderRadius: 8, backgroundColor: RED, scale: dot }}
          />
          <Text
            style={{
              fontFamily: "GeistMono",
              fontSize: 20,
              letterSpacing: 4,
              color: INK,
              fontVariantNumeric: "tabular-nums",
            }}
          >
            WGHT {padNumber(120 + 780 * wave, { width: 3 })} · WDTH{" "}
            {padNumber(62 + 63 * wave, { width: 3 })}
          </Text>
        </View>
        <Text style={{ ...LABEL, color: INK }}>ARCHIVO · 100—900 · 62—125</Text>
      </View>
    </View>
  );
}
