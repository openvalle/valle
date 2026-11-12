// Glitch: a word that will not hold. It arrives torn into bands that slide apart, its colours
// split and its edges boil; then the picture locks and the word reads SIGNAL. Valle's own
// filters, on one layer: displacement, chromatic aberration, lens distortion and film grain.
// Designed at 1920 x 1080, 30 fps, 5 s.

export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

// The fonts are Google Fonts' files through jsDelivr, at a pinned commit, under the SIL Open Font License 1.1.
export const controls = {
  resources: {
    Anton: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/anton/Anton-Regular.ttf",
    }),
    GeistMono: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geistmono/GeistMono%5Bwght%5D.ttf",
    }),
  },
};

const BLACK = "#060608";
const ACID = "#C8FF3E";
const PINK = "#FF3E8A";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
// The word is drawn in bands, each free to slide sideways.
const BANDS = 18;
const WORD = { top: 250, height: 580 };
const BAND = WORD.height / BANDS;
const SLICES = Array.from({ length: BANDS }, (_, i) => ({ key: `band-${i}`, index: i }));
// When the picture locks.
const LOCK = 2.9;

// How badly the picture tears, from 0 to 1: on arrival, in two bursts, at the lock, and one
// last flicker after it.
function trouble(seconds) {
  const burst = (at, length) =>
    interpolate(seconds, [at, at + 0.04, at + length], [0, 1, 0], { easing: "easeOutCubic" });
  return Math.min(
    1,
    interpolate(seconds, [0, 0.9], [1, 0.12], { easing: "easeOutCubic" }) *
      (seconds < LOCK ? 1 : 0) +
      burst(1.45, 0.4) +
      burst(2.2, 0.3) +
      1.2 * burst(LOCK - 0.25, 0.5) +
      0.5 * burst(3.9, 0.14),
  );
}

function Word({ children, color, shown }) {
  const { seconds } = useCurrentTime();
  const amount = trouble(seconds);
  // The tearing changes fifteen times a second, not every frame.
  const beat = Math.floor(seconds * 15);
  return (
    <View key="word" className="absolute inset-0" style={{ opacity: shown }}>
      {SLICES.map((slice) => {
        const pull = 2 * noise2d(9, slice.index * 3.7, beat * 1.31) - 1;
        // Most bands hold; the few the noise pulls hardest go far.
        const far = Math.max(0, Math.abs(pull) - 0.18);
        const shift = amount * 1500 * far * (pull < 0 ? -1 : 1);
        return (
          <View
            key={slice.key}
            className="absolute overflow-hidden"
            style={{
              left: 0,
              top: WORD.top + slice.index * BAND,
              width: 1920,
              height: BAND + 0.5,
              translate: `${shift}px 0px`,
            }}
          >
            <Text
              className="absolute"
              style={{
                left: 0,
                top: -slice.index * BAND,
                width: 1920,
                height: WORD.height,
                textAlign: "center",
                fontFamily: "Anton",
                fontSize: 520,
                lineHeight: `${WORD.height}px`,
                letterSpacing: 6,
                color,
              }}
            >
              {children}
            </Text>
          </View>
        );
      })}
    </View>
  );
}

export default function Glitch() {
  const { seconds } = useCurrentTime();
  const amount = trouble(seconds);
  const beat = Math.floor(seconds * 15);
  const locked = seconds >= LOCK ? 1 : 0;
  const meta = interpolate(seconds, [0.3, 0.9], [0, 1]);
  const settled = interpolate(seconds, [LOCK + 0.1, LOCK + 0.6], [0, 1], {
    easing: "easeOutCubic",
  });
  const frames = Math.floor(seconds * 30);
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: BLACK }}>
      <View
        key="screen"
        className="absolute inset-0"
        style={{
          filter: `displacement(${beat} 0.002 0.05 ${5 * (1 - locked) + 26 * amount}px 2 turbulence) chromatic-aberration(${2 + 3 * (1 - locked) + 90 * amount}px) lens-distortion(0.06 0) film-grain(${frames} 0.16 2px)`,
        }}
      >
        <View
          key="glow"
          className="absolute inset-0"
          style={{
            backgroundImage: `radial-gradient(ellipse 1100px 560px at 960px 540px, ${
              locked ? "#C8FF3E24" : "#FF3E8A24"
            } 0%, #06060800 100%)`,
          }}
        />
        <Word color={PINK} shown={1 - locked}>
          NOISE
        </Word>
        <Word color={ACID} shown={locked}>
          SIGNAL
        </Word>
        <View
          key="bar"
          className="absolute"
          style={{
            left: 0,
            top: 0,
            width: 1920,
            height: 90,
            backgroundColor: "#FFFFFF",
            opacity: 0.1 + 0.25 * amount,
            translate: `0px ${mod(seconds * 520, 1260) - 90}px`,
          }}
        />
      </View>

      <View
        key="lines"
        className="absolute inset-0"
        style={{
          backgroundImage: "linear-gradient(#00000000 50%, #00000066 50%)",
          backgroundSize: "100% 6px",
        }}
      />
      <View
        key="vignette"
        className="absolute inset-0"
        style={{
          backgroundImage:
            "radial-gradient(ellipse 1300px 760px at 960px 540px, #00000000 55%, #000000B3 100%)",
        }}
      />

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: "#F4F4F0" }}>N° 09 — GLITCH</Text>
        <Text style={{ ...LABEL, color: "#F4F4F0", fontVariantNumeric: "tabular-nums" }}>
          CH 09 · 00:00:{padNumber(Math.floor(seconds), { width: 2 })}:
          {padNumber(frames - 30 * Math.floor(seconds), { width: 2 })}
        </Text>
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
              backgroundColor: locked ? ACID : PINK,
              opacity: locked ? 1 : 0.35 + 0.65 * (beat - 2 * Math.floor(beat / 2)),
            }}
          />
          <Text style={{ ...LABEL, color: "#F4F4F0" }}>
            TEAR {padNumber(100 * amount, { width: 3 })}
          </Text>
          <Text style={{ ...LABEL, color: ACID, opacity: settled }}>LOCKED</Text>
        </View>
        <Text style={{ ...LABEL, color: "#F4F4F0" }}>FOUR FILTERS · ONE LAYER</Text>
      </View>
    </View>
  );
}
