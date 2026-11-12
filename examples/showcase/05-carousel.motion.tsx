// Carousel: sixteen cards in a ring, each a small picture of its own. The camera starts close
// on one card, pulls back until the whole ring turns in perspective over its reflection, and
// the ring stops with a card facing the viewer. CSS 3D only.
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
const ORANGE = "#FF9F45";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const COUNT = 16;
const STEP = 360 / COUNT;
const PANEL = { width: 300, height: 430 };
const RING = 820;
const CENTER = { x: 960, y: 412 };
const PERSPECTIVE = 2600;

// Six kinds of card, going around the ring: the ground it is painted on, the colour its back
// and its reflection take, and the colour of its captions.
const KINDS = [
  {
    name: "LIGHT",
    tint: "#7A3BFF",
    ink: CREAM,
    ground:
      "radial-gradient(circle 210px at 236px 120px, #FF9F45 0%, #FF9F4500 100%), radial-gradient(circle 260px at 50px 330px, #7A3BFF 0%, #7A3BFF00 100%), linear-gradient(#150B33, #150B33)",
  },
  { name: "TYPE", tint: "#F4EEE4", ink: "#16140F", ground: "linear-gradient(#F4EEE4, #E6DFD2)" },
  { name: "DATA", tint: "#FF9F45", ink: CREAM, ground: "linear-gradient(#1B1820, #121016)" },
  { name: "LIQUID", tint: "#1B2CF2", ink: CREAM, ground: "linear-gradient(#2536FF, #1420C4)" },
  { name: "SEEDS", tint: "#F2416B", ink: CREAM, ground: "linear-gradient(#17111C, #0B0A0D)" },
  { name: "CUT", tint: "#22C7A9", ink: CREAM, ground: "linear-gradient(#10201F, #0C1415)" },
];

const PANELS = Array.from({ length: COUNT }, (_, i) => ({
  key: `panel-${i}`,
  index: i,
  kind: i % KINDS.length,
  number: String(i + 1).padStart(2, "0"),
}));

const BARS = [0.34, 0.52, 0.44, 0.7, 0.62, 0.86, 1].map((value, i) => ({
  key: `bar-${i}`,
  value,
  i,
}));
const DROPS = [
  { key: "a", x: 78, y: 150, size: 150 },
  { key: "b", x: 176, y: 206, size: 104 },
  { key: "c", x: 120, y: 268, size: 82 },
  { key: "d", x: 226, y: 122, size: 44 },
  { key: "e", x: 62, y: 322, size: 30 },
];
const SEEDS = Array.from({ length: 120 }, (_, i) => {
  const u = (i + 0.5) / 120;
  const a = (i * 137.50776 * Math.PI) / 180;
  return {
    key: `seed-${i}`,
    x: 150 + 112 * Math.sqrt(u) * Math.cos(a),
    y: 212 + 112 * Math.sqrt(u) * Math.sin(a),
    r: 2 + 3.4 * Math.sqrt(u),
    color: interpolate(u, [0, 0.4, 1], ["#FFE9A8", "#FF9F45", "#F2416B"], { colorSpace: "oklch" }),
  };
});
const CLIPS = [
  { key: "a", row: 0, x: 0, w: 96, color: "#22C7A9" },
  { key: "b", row: 0, x: 104, w: 152, color: "#17907B" },
  { key: "c", row: 1, x: 40, w: 128, color: "#FF9F45" },
  { key: "d", row: 1, x: 176, w: 80, color: "#C9742A" },
  { key: "e", row: 2, x: 0, w: 60, color: "#7A3BFF" },
  { key: "f", row: 2, x: 68, w: 188, color: "#5B2CC4" },
  { key: "g", row: 3, x: 20, w: 216, color: "#F4EEE44D" },
];

// What a card shows, by its kind: fixed pictures, drawn once.
function Art({ kind }) {
  if (kind === 0) {
    return (
      <Text
        className="absolute"
        style={{
          left: 22,
          top: 262,
          fontFamily: "Instrument",
          fontStyle: "italic",
          fontSize: 104,
          lineHeight: 1,
          letterSpacing: -2,
          color: CREAM,
        }}
      >
        Light
      </Text>
    );
  }
  if (kind === 1) {
    return (
      <Text
        className="absolute"
        style={{
          left: 0,
          top: 96,
          width: PANEL.width,
          textAlign: "center",
          fontFamily: "Instrument",
          fontSize: 232,
          lineHeight: 1,
          letterSpacing: -8,
          color: "#16140F",
        }}
      >
        A<Span style={{ fontStyle: "italic", color: "#F03A17" }}>g</Span>
      </Text>
    );
  }
  if (kind === 2) {
    return (
      <svg className="absolute" style={{ left: 22, top: 110 }} width={256} height={240}>
        {BARS.map((bar) => (
          <rect
            key={bar.key}
            x={bar.i * 37}
            y={232 - 216 * bar.value}
            width={26}
            height={216 * bar.value}
            rx={5}
            fill={interpolate(bar.value, [0.3, 1], ["#6B3BFF", "#FF9F45"], { colorSpace: "oklch" })}
          />
        ))}
        <line x1={0} y1={236} x2={256} y2={236} stroke="#F4EEE44D" strokeWidth={1.5} />
      </svg>
    );
  }
  if (kind === 3) {
    return (
      <View className="absolute inset-0">
        {DROPS.map((drop) => (
          <View
            key={drop.key}
            className="absolute"
            style={{
              left: drop.x - drop.size / 2,
              top: drop.y - drop.size / 2,
              width: drop.size,
              height: drop.size,
              borderRadius: drop.size / 2,
              backgroundImage: "linear-gradient(160deg, #FFF4E6, #FFC9A3)",
            }}
          />
        ))}
      </View>
    );
  }
  if (kind === 4) {
    return (
      <svg className="absolute inset-0" width={PANEL.width} height={PANEL.height}>
        {SEEDS.map((seed) => (
          <circle key={seed.key} cx={seed.x} cy={seed.y} r={seed.r} fill={seed.color} />
        ))}
      </svg>
    );
  }
  return (
    <svg className="absolute" style={{ left: 22, top: 128 }} width={256} height={210}>
      {CLIPS.map((clip) => (
        <rect
          key={clip.key}
          x={clip.x}
          y={clip.row * 50}
          width={clip.w}
          height={38}
          rx={8}
          fill={clip.color}
        />
      ))}
      <line x1={150} y1={-6} x2={150} y2={204} stroke={CREAM} strokeWidth={2} />
      <circle cx={150} cy={-2} r={6} fill={CREAM} />
    </svg>
  );
}

// How far the ring has turned: it starts with card 08 at the front and stops on card 01.
function spin(seconds) {
  return interpolate(seconds, [0.15, 4.3], [-7 * STEP, 0], { easing: "easeInOutCubic" });
}

function Panel({ panel }) {
  const { seconds } = useCurrentTime();
  const kind = KINDS[panel.kind];
  const turn = panel.index * STEP + spin(seconds);
  // 1 where the face looks at the viewer, -1 where its back does.
  const facing = Math.cos((turn * Math.PI) / 180);
  const shade = interpolate(facing, [0, 1], [0.6, 0]);
  // The card the ring stops on steps out of it.
  const lift = panel.index === 0 ? spring(seconds - 4.2, { duration: 0.6, bounce: 0.25 }) : 0;
  return (
    <View
      className="absolute"
      style={{
        left: CENTER.x - PANEL.width / 2,
        top: CENTER.y - PANEL.height / 2,
        width: PANEL.width,
        height: PANEL.height,
        transformStyle: "preserve-3d",
        transform: `rotateY(${turn}deg) translateZ(${RING + 50 * lift}px)`,
      }}
    >
      <View
        key="front"
        className="absolute inset-0 overflow-hidden"
        style={{
          borderRadius: 22,
          backgroundImage: kind.ground,
          backfaceVisibility: "hidden",
        }}
      >
        <Art kind={panel.kind} />
        <Text
          className="absolute"
          style={{
            left: 22,
            top: 22,
            fontFamily: "GeistMono",
            fontSize: 14,
            letterSpacing: 3,
            color: kind.ink,
          }}
        >
          {kind.name}
        </Text>
        <Text
          className="absolute"
          style={{
            right: 22,
            bottom: 20,
            fontFamily: "GeistMono",
            fontSize: 14,
            letterSpacing: 3,
            color: kind.ink,
            opacity: 0.7,
          }}
        >
          {panel.number} / {COUNT}
        </Text>
        <View
          key="shade"
          className="absolute inset-0"
          style={{ backgroundColor: NIGHT, opacity: shade }}
        />
      </View>
      <View
        key="back"
        className="absolute inset-0 overflow-hidden"
        style={{
          borderRadius: 22,
          backgroundImage: kind.ground,
          border: "1.5px solid #F4EEE433",
          backfaceVisibility: "hidden",
          transform: "rotateY(180deg)",
          filter: "blur(3px)",
        }}
      >
        <View
          key="shade"
          className="absolute inset-0"
          style={{
            backgroundColor: NIGHT,
            opacity: interpolate(facing, [-1, 0], [0.74, 0.86]),
          }}
        />
      </View>
    </View>
  );
}

// A card's reflection in the floor: its colour only, mirrored about its foot and fading away.
function Reflection({ panel }) {
  const { seconds } = useCurrentTime();
  const kind = KINDS[panel.kind];
  const turn = panel.index * STEP + spin(seconds);
  const facing = Math.cos((turn * Math.PI) / 180);
  return (
    <View
      className="absolute"
      style={{
        left: CENTER.x - PANEL.width / 2,
        top: CENTER.y - PANEL.height / 2,
        width: PANEL.width,
        height: PANEL.height,
        borderRadius: 22,
        backgroundColor: kind.tint,
        opacity: interpolate(facing, [-1, 1], [0.04, 0.22]),
        maskImage: "linear-gradient(to top, #000000 0%, #00000000 48%)",
        transform: `rotateY(${turn}deg) translateZ(${RING}px) translateY(${PANEL.height + 14}px) scaleY(-1)`,
      }}
    />
  );
}

export default function Carousel() {
  const { seconds } = useCurrentTime();
  // The camera: close on one card, then back and a little above.
  const push = interpolate(seconds, [0.2, 2.6], [1040, 0], { easing: "easeInOutCubic" });
  const tilt = interpolate(seconds, [0.4, 2.8], [0, -11], { easing: "easeInOutCubic" });
  const meta = interpolate(seconds, [1.9, 2.7], [0, 1], { easing: "easeOutCubic" });
  const title = interpolate(seconds, [2.9, 3.8], [0, 1], { easing: "easeOutCubic" });
  const pool = interpolate(seconds, [1.0, 2.8], [0, 1], { easing: "easeOutCubic" });
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: NIGHT }}>
      <View
        key="glow"
        className="absolute inset-0"
        style={{
          backgroundImage:
            "radial-gradient(ellipse 1100px 560px at 960px 400px, #3A1D5C73 0%, #0B0A0D00 100%)",
        }}
      />
      <View
        key="pool"
        className="absolute"
        style={{
          left: 60,
          top: 600,
          width: 1800,
          height: 420,
          opacity: pool,
          backgroundImage: "radial-gradient(ellipse at center, #6B3BFF38 0%, #0B0A0D00 66%)",
        }}
      />

      <View
        key="stage"
        className="absolute"
        style={{
          left: 0,
          top: 0,
          width: 1920,
          height: 1080,
          perspective: PERSPECTIVE,
          perspectiveOrigin: "960px 392px",
        }}
      >
        <View
          key="ring"
          className="absolute"
          style={{
            left: 0,
            top: 0,
            width: 1920,
            height: 1080,
            transformStyle: "preserve-3d",
            transformOrigin: `${CENTER.x}px ${CENTER.y}px`,
            transform: `translateZ(${push}px) rotateX(${tilt}deg)`,
          }}
        >
          {PANELS.map((panel) => (
            <Reflection key={`mirror-${panel.index}`} panel={panel} />
          ))}
          {PANELS.map((panel) => (
            <Panel key={panel.key} panel={panel} />
          ))}
        </View>
      </View>

      <View
        key="fade"
        className="absolute inset-0"
        style={{
          opacity: pool,
          backgroundImage: "linear-gradient(180deg, #0B0A0D00 80%, #0B0A0DCC 100%)",
        }}
      />

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: CREAM }}>N° 05 — CAROUSEL</Text>
        <Text style={{ ...LABEL, color: CREAM }}>16 CARDS · PERSPECTIVE 2600</Text>
      </View>

      <View
        key="foot"
        className="absolute flex items-end justify-between"
        style={{
          left: 96,
          right: 96,
          bottom: 64,
          opacity: title,
          translate: `0px ${(1 - title) * 18}px`,
        }}
      >
        <Text
          style={{
            fontFamily: "Instrument",
            fontSize: 96,
            lineHeight: 0.95,
            letterSpacing: -2,
            color: CREAM,
          }}
        >
          In the <Span style={{ fontStyle: "italic", color: ORANGE }}>round.</Span>
        </Text>
        <Text style={{ ...LABEL, color: CREAM }}>CSS 3D · NO MESHES</Text>
      </View>
    </View>
  );
}
