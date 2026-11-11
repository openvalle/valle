// Carousel: fourteen panels open from one stack into a ring and turn in perspective, each face
// lit by how far it turns toward the viewer. CSS 3D only.
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

const STAGE = "#ECE8E1";
const INK = "#16140F";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const COUNT = 14;
const PANEL = { width: 214, height: 336 };
const RING = 560;
const CENTER = { x: 960, y: 452 };

// The ring's colours go once around: vermilion, orange, pink, violet, cobalt and back.
const STOPS = [0, 0.2, 0.4, 0.6, 0.8, 1];
const HUES = ["#FF4B2B", "#FF8A1F", "#FF6FA3", "#6A3BFF", "#2340F5", "#FF4B2B"];
const PANELS = Array.from({ length: COUNT }, (_, i) => {
  const color = interpolate(i / COUNT, STOPS, HUES, { colorSpace: "oklch" });
  return {
    key: `panel-${i}`,
    index: i,
    number: String(i + 1).padStart(2, "0"),
    // Lit from above: a little lighter at the top, a little darker at the foot.
    face: `linear-gradient(180deg, #FFFFFF33 0%, #FFFFFF00 42%, #16140F24 100%), linear-gradient(${color}, ${color})`,
  };
});

function Panel({ panel }) {
  const { seconds } = useCurrentTime();
  const open = interpolate(
    seconds,
    [0.5 + panel.index * 0.035, 1.9 + panel.index * 0.035],
    [0, 1],
    {
      easing: "easeOutExpo",
    },
  );
  const spin =
    34 * seconds + 40 * interpolate(seconds, [0.4, 2.4], [0, 1], { easing: "easeOutCubic" });
  // The stack fans out both ways from the front, so the ring closes at the back.
  const turn = ((panel.index - (COUNT - 1) / 2) / COUNT) * 360 * open + spin;
  // 1 where the face looks at the viewer, -1 where its back does.
  const facing = Math.cos((turn * Math.PI) / 180);
  const shade = interpolate(facing, [-1, 0, 1], [0.5, 0.34, 0]);
  return (
    <View
      className="absolute"
      style={{
        left: CENTER.x - PANEL.width / 2,
        top: CENTER.y - PANEL.height / 2,
        width: PANEL.width,
        height: PANEL.height,
        transformStyle: "preserve-3d",
        transform: `rotateY(${turn}deg) translateZ(${RING * open}px)`,
      }}
    >
      <View
        key="front"
        className="absolute flex flex-col justify-between overflow-hidden"
        style={{
          left: 0,
          top: 0,
          width: PANEL.width,
          height: PANEL.height,
          padding: 20,
          borderRadius: 18,
          backgroundImage: panel.face,
          backfaceVisibility: "hidden",
        }}
      >
        <Text
          style={{ fontFamily: "GeistMono", fontSize: 15, letterSpacing: 3, color: "#FFFFFFE6" }}
        >
          FRAME
        </Text>
        <Text
          style={{
            fontFamily: "Instrument",
            fontSize: 136,
            lineHeight: 0.8,
            letterSpacing: -4,
            color: "#FFFFFF",
          }}
        >
          {panel.number}
        </Text>
        <View
          key="shade"
          className="absolute inset-0"
          style={{ backgroundColor: "#16140F", opacity: shade }}
        />
      </View>
      <View
        key="back"
        className="absolute"
        style={{
          left: 0,
          top: 0,
          width: PANEL.width,
          height: PANEL.height,
          borderRadius: 18,
          backgroundColor: "#FBF9F5",
          border: "1.5px solid #16140F1A",
          backfaceVisibility: "hidden",
          transform: "rotateY(180deg)",
        }}
      >
        <View
          key="shade"
          className="absolute inset-0"
          style={{ borderRadius: 18, backgroundColor: "#16140F", opacity: 0.2 - 0.3 * shade }}
        />
      </View>
    </View>
  );
}

export default function Carousel() {
  const { seconds } = useCurrentTime();
  const tilt = interpolate(seconds, [0.5, 2.6], [0, -13], { easing: "easeInOutCubic" });
  const meta = interpolate(seconds, [0.7, 1.5], [0, 1], { easing: "easeOutCubic" });
  const title = interpolate(seconds, [1.6, 2.5], [0, 1], { easing: "easeOutCubic" });
  const floor = interpolate(seconds, [0.6, 2.2], [0, 1], { easing: "easeOutCubic" });
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: STAGE }}>
      <View
        key="light"
        className="absolute inset-0"
        style={{
          backgroundImage:
            "radial-gradient(ellipse 60% 55% at 50% 40%, #FFFFFF 0%, #ECE8E100 100%), linear-gradient(180deg, #ECE8E100 55%, #D9D3C8 100%)",
        }}
      />
      <View
        key="floor"
        className="absolute"
        style={{
          left: 160,
          top: 640,
          width: 1600,
          height: 240,
          opacity: floor,
          scale: `${0.4 + 0.6 * floor} 1`,
          backgroundImage: "radial-gradient(ellipse at center, #16140F47 0%, #16140F00 68%)",
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
          perspective: 2200,
          perspectiveOrigin: "960px 400px",
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
            transform: `rotateX(${tilt}deg)`,
          }}
        >
          {PANELS.map((panel) => (
            <Panel key={panel.key} panel={panel} />
          ))}
        </View>
      </View>

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: INK }}>N° 05 — CAROUSEL</Text>
        <Text style={{ ...LABEL, color: INK }}>14 PANELS · PERSPECTIVE 2200</Text>
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
            color: INK,
          }}
        >
          In the <Span style={{ fontStyle: "italic" }}>round.</Span>
        </Text>
        <Text style={{ ...LABEL, color: INK }}>CSS 3D · NO MESHES</Text>
      </View>
    </View>
  );
}
