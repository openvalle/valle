// Dawn: a valley cut from six sheets of paper. The sheets rise into place, the sky turns from
// night to morning behind them, the stars go out and the sun comes up over the far ridge.
// Every ridge is one line of noise, drawn once; only colours and positions follow the time.
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

const CREAM = "#FFF6E9";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const NIGHT_SKY = "linear-gradient(180deg, #0B0D2B 0%, #1F1B4D 55%, #40306B 100%)";
const DAY_SKY = "linear-gradient(180deg, #5B6FD6 0%, #F2A5A0 46%, #FFD9A0 78%, #FFEFC9 100%)";
const SUN = { x: 1240, from: 760, to: 372, size: 190 };

// The sheets, far to near: where each ridge stands, how tall its hills are and how often they
// come, and its colour by night and by day.
const SHEETS = [
  { base: 650, tall: 250, often: 3.4, seed: 3, night: "#3A3470", day: "#F6C3A8" },
  { base: 705, tall: 240, often: 4.6, seed: 8, night: "#302B63", day: "#EC9A8E" },
  { base: 760, tall: 210, often: 3.0, seed: 15, night: "#262256", day: "#CF6F84" },
  { base: 835, tall: 200, often: 3.8, seed: 21, night: "#1D1A47", day: "#99527F" },
  { base: 915, tall: 170, often: 3.2, seed: 34, night: "#151338", day: "#5E3A70" },
  { base: 1000, tall: 130, often: 4.6, seed: 55, night: "#0D0C27", day: "#33254F" },
].map((sheet, index) => {
  // The ridge, and on the near sheets the pines along it.
  // The far sheets are peaks, where the noise crosses its middle; the near ones are hills.
  const height = (x) => {
    const n = noise1d(sheet.seed, (x / 1920) * sheet.often);
    // They stand to the right, under the sun, and leave the sky on the left to the title.
    const right = Math.min(1, Math.max(0, (x - 420) / 900));
    const stand = 0.3 + 0.7 * right * right * (3 - 2 * right);
    const shape = index < 2 ? stand * (1 - Math.abs(2 * n - 1)) ** 1.6 : n;
    const detail = noise1d(sheet.seed + 1, (x / 1920) * sheet.often * 3.3);
    return sheet.base - sheet.tall * shape - 0.2 * sheet.tall * detail;
  };
  let d = `M-40 1300L-40 ${height(-40)}`;
  for (let x = -20; x <= 1960; x += 20) d += `L${x} ${height(x)}`;
  d += "L1960 1300Z";
  let pines = "";
  if (index >= 3) {
    const sizes = seededRandom(sheet.seed, 60, [0.55, 1]);
    const places = seededRandom(sheet.seed + 7, 60, [0, 1920]);
    for (let i = 0; i < (index === 3 ? 26 : 44); i++) {
      const tall = (index - 2) * 30 * sizes[i];
      const x = places[i];
      const y = height(x) + 6;
      pines += `M${x - tall * 0.3} ${y}L${x} ${y - tall}L${x + tall * 0.3} ${y}Z`;
    }
  }
  return { ...sheet, key: `sheet-${index}`, index, d: d + pines };
});

const STARS = seededRandom(4, 90).map((u, i) => ({
  key: `star-${i}`,
  x: 1920 * u,
  y: 560 * seededRandom(5, 90)[i],
  r: 1 + 1.6 * seededRandom(6, 90)[i],
  twinkle: 6.3 * seededRandom(7, 90)[i],
}));
const BIRDS = [
  { key: "a", x: 700, y: 290, size: 1 },
  { key: "b", x: 770, y: 330, size: 0.8 },
  { key: "c", x: 650, y: 342, size: 0.7 },
];

export default function Dawn() {
  const { seconds } = useCurrentTime();
  // Night to morning.
  const day = interpolate(seconds, [0.9, 3.9], [0, 1], { easing: "easeInOutSine" });
  const sun = interpolate(seconds, [1.0, 4.2], [SUN.from, SUN.to], { easing: "easeOutCubic" });
  const meta = interpolate(seconds, [0.6, 1.4], [0, 1], { easing: "easeOutCubic" });
  const title = interpolate(seconds, [2.9, 3.8], [0, 1], { easing: "easeOutCubic" });
  const ink = interpolate(day, [0, 1], [CREAM, "#2A1B3D"]);
  return (
    <View className="relative h-full w-full" style={{ backgroundImage: NIGHT_SKY }}>
      <View
        key="day"
        className="absolute inset-0"
        style={{ opacity: day, backgroundImage: DAY_SKY }}
      />

      <svg key="stars" className="absolute inset-0" width={1920} height={1080}>
        {STARS.map((star) => (
          <circle
            key={star.key}
            cx={star.x}
            cy={star.y}
            r={star.r}
            fill={CREAM}
            opacity={(1 - day) * (0.55 + 0.45 * Math.sin(seconds * 3 + star.twinkle))}
          />
        ))}
      </svg>

      <View
        key="halo"
        className="absolute"
        style={{
          left: SUN.x - 700,
          top: -700,
          width: 1400,
          height: 1400,
          opacity: day,
          translate: `0px ${sun}px`,
          backgroundImage: "radial-gradient(circle, #FFF1C2B3 0%, #FFD08A40 34%, #FFD08A00 68%)",
        }}
      />
      <View
        key="sun"
        className="absolute"
        style={{
          left: SUN.x - SUN.size / 2,
          top: -SUN.size / 2,
          width: SUN.size,
          height: SUN.size,
          borderRadius: SUN.size / 2,
          translate: `0px ${sun}px`,
          backgroundImage:
            "radial-gradient(circle at 40% 36%, #FFFDF2 0%, #FFE9A8 70%, #FFD27A 100%)",
        }}
      />

      <svg key="birds" className="absolute inset-0" width={1920} height={1080}>
        {BIRDS.map((bird) => {
          // Each wing beats a little out of step with the others.
          const flap = 9 * Math.sin(seconds * 7 + bird.x);
          const x = bird.x + seconds * 26;
          const y = bird.y - seconds * 9 + 5 * Math.sin(seconds * 2 + bird.y);
          const w = 22 * bird.size;
          return (
            <path
              key={bird.key}
              d={`M${x - w} ${y - flap * bird.size}Q${x - w / 2} ${y - 7 * bird.size} ${x} ${y}Q${x + w / 2} ${y - 7 * bird.size} ${x + w} ${y - flap * bird.size}`}
              fill="none"
              stroke="#2A1B3D"
              strokeWidth={3 * bird.size}
              strokeLinecap="round"
              opacity={interpolate(day, [0.35, 0.8], [0, 0.8])}
            />
          );
        })}
      </svg>

      <svg key="valley" className="absolute inset-0" width={1920} height={1080}>
        <defs>
          <linearGradient id="haze" gradientUnits="userSpaceOnUse" x1={0} y1={520} x2={0} y2={1080}>
            <stop offset={0} stopColor="#FFFFFF00" />
            <stop offset={1} stopColor="#FFFFFF38" />
          </linearGradient>
        </defs>
        {SHEETS.map((sheet) => {
          // The sheets come up from below, the far ones first, and the near ones move the most
          // as the view settles.
          const up = interpolate(
            seconds,
            [0.1 + sheet.index * 0.12, 1.5 + sheet.index * 0.12],
            [1, 0],
            { easing: "easeOutExpo" },
          );
          const settle = interpolate(seconds, [1.2, 5], [1, 0], { easing: "easeOutCubic" });
          const y = up * (520 - sheet.index * 30) + settle * sheet.index * 9;
          return (
            <g key={sheet.key} transform={`translate(0 ${y})`}>
              <path d={sheet.d} fill="#0A081E" opacity={0.16} transform="translate(0 -9)" />
              <path
                d={sheet.d}
                fill={interpolate(day, [0, 1], [sheet.night, sheet.day], { colorSpace: "oklch" })}
              />
              <path d={sheet.d} fill="url(#haze)" opacity={day * (1 - sheet.index / 6)} />
            </g>
          );
        })}
      </svg>

      <View
        key="grain"
        className="absolute inset-0"
        style={{ filter: "film-grain(5 0.09 1.5px)" }}
      />

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: ink }}>N° 10 — DAWN</Text>
        <Text style={{ ...LABEL, color: ink }}>SIX SHEETS · ONE SUN</Text>
      </View>

      <View
        key="title"
        className="absolute flex flex-col"
        style={{
          left: 96,
          top: 150,
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
            color: "#2A1B3D",
          }}
        >
          Day,
        </Text>
        <Text
          style={{
            fontFamily: "Instrument",
            fontStyle: "italic",
            fontSize: 148,
            lineHeight: 0.95,
            letterSpacing: -3,
            color: "#2A1B3D",
          }}
        >
          breaking.
        </Text>
      </View>

      <View
        key="foot"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, bottom: 64, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: CREAM }}>CUT FROM NOISE</Text>
        <Text style={{ ...LABEL, color: CREAM }}>SVG · NO IMAGES</Text>
      </View>
    </View>
  );
}
