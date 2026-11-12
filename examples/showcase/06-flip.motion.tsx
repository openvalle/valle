// Flip: a wall of 144 tiles, each a slice of one picture. A wave turns them over along the
// diagonal to a second picture and its words, another runs out from the middle to a third, and
// the gaps between the tiles close until the wall is one sheet. CSS 3D only.
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
const INK = "#16140F";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const TILE = 120;
const COLUMNS = 16;
const ROWS = 9;
const GAP = 10;

// The three pictures, each the size of the frame: a tile shows the part of one behind it.
const SLATE = "linear-gradient(160deg, #1D1826 0%, #100E15 100%)";
const COOL =
  "radial-gradient(circle 900px at 380px 240px, #2536FF 0%, #2536FF00 100%), radial-gradient(circle 1000px at 1560px 860px, #7A3BFF 0%, #7A3BFF00 100%), radial-gradient(circle 620px at 1080px 380px, #22C7A9 0%, #22C7A900 100%), linear-gradient(#0E0B2A, #0E0B2A)";
const WARM =
  "radial-gradient(circle 880px at 1500px 220px, #FFC46B 0%, #FFC46B00 100%), radial-gradient(circle 1000px at 300px 900px, #6B3BFF 0%, #6B3BFF00 100%), radial-gradient(circle 760px at 900px 560px, #F2416B 0%, #F2416B00 100%), linear-gradient(#FF7A2E, #FF7A2E)";

const TILES = Array.from({ length: COLUMNS * ROWS }, (_, i) => {
  const column = i % COLUMNS;
  const row = Math.floor(i / COLUMNS);
  const dx = column - (COLUMNS - 1) / 2;
  const dy = row - (ROWS - 1) / 2;
  return {
    key: `tile-${i}`,
    x: column * TILE,
    y: row * TILE,
    // When each wave reaches it, from 0 to 1: the first along the diagonal, the second outward.
    first: (column + row) / (COLUMNS + ROWS - 2),
    second: Math.hypot(dx, dy) / Math.hypot((COLUMNS - 1) / 2, (ROWS - 1) / 2),
    // Whether the words cross it: the others carry none, which is most of what a frame costs.
    worded: row >= 2 && row <= 6 && column >= 2 && column <= 13,
  };
});

// The words of a picture, set once over the whole frame.
function Words({ children, italic, color, shown = true }) {
  return (
    <Text
      className="absolute"
      style={{
        left: 0,
        top: 250,
        width: 1920,
        textAlign: "center",
        fontFamily: "Instrument",
        fontStyle: italic ? "italic" : "normal",
        fontSize: 520,
        lineHeight: 1,
        letterSpacing: -14,
        color,
        opacity: shown ? 1 : 0,
      }}
    >
      {children}
    </Text>
  );
}

// A face of a tile: its part of a picture and of the picture's words, darker as it turns away.
function Face({ picture, x, y, shade, back, children }) {
  return (
    <View
      className={children ? "absolute inset-0 overflow-hidden" : "absolute inset-0"}
      style={{
        backgroundImage: picture,
        backgroundSize: "1920px 1080px",
        backgroundPosition: `${-x}px ${-y}px`,
        backfaceVisibility: "hidden",
        transform: back ? "rotateY(180deg)" : "none",
      }}
    >
      {children ? (
        <View className="absolute" style={{ left: -x, top: -y, width: 1920, height: 1080 }}>
          {children}
        </View>
      ) : null}
      <View className="absolute inset-0" style={{ backgroundColor: NIGHT, opacity: shade }} />
    </View>
  );
}

function Tile({ tile }) {
  const { seconds } = useCurrentTime();
  const first = interpolate(seconds, [0.5 + tile.first * 1.3, 1.2 + tile.first * 1.3], [0, 1], {
    easing: "easeInOutCubic",
  });
  const second = interpolate(seconds, [2.7 + tile.second * 1.0, 3.4 + tile.second * 1.0], [0, 1], {
    easing: "easeInOutCubic",
  });
  const turn = 180 * (first + second);
  // 0 flat to the viewer, 1 edge on.
  const edge = 1 - Math.abs(Math.cos((turn * Math.PI) / 180));
  const lift = 90 * Math.sin(Math.PI * first) + 90 * Math.sin(Math.PI * second);
  // The gaps close once every tile shows the last picture.
  const close = interpolate(seconds, [4.15, 4.7], [0, 1], { easing: "easeInOutCubic" });
  const size = (TILE - GAP + GAP * close) / TILE;
  // The front is the first picture until it is turned away, then the last.
  const late = seconds >= 2.5;
  return (
    <View
      className="absolute"
      style={{
        left: tile.x,
        top: tile.y,
        width: TILE,
        height: TILE,
        transformStyle: "preserve-3d",
        transform: `translateZ(${lift}px) rotateY(${turn}deg) scale(${size})`,
      }}
    >
      <Face key="front" picture={late ? WARM : SLATE} x={tile.x} y={tile.y} shade={0.6 * edge}>
        {tile.worded ? (
          <Words italic color={INK} shown={late}>
            over.
          </Words>
        ) : null}
      </Face>
      <Face key="back" picture={COOL} x={tile.x} y={tile.y} shade={0.6 * edge} back>
        {tile.worded ? <Words color={CREAM}>Turn it</Words> : null}
      </Face>
    </View>
  );
}

export default function Flip() {
  const { seconds } = useCurrentTime();
  const meta = interpolate(seconds, [0.3, 1.0], [0, 1], { easing: "easeOutCubic" });
  // One sheet over the wall once its gaps have closed, so no seam is left between tiles.
  const sheet = interpolate(seconds, [4.62, 4.78], [0, 1]);
  // The captions go dark as the wall turns light.
  const ink = interpolate(seconds, [3.4, 4.2], [CREAM, INK]);
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: NIGHT }}>
      <View
        key="wall"
        className="absolute"
        style={{
          left: 0,
          top: 0,
          width: 1920,
          height: 1080,
          perspective: 1800,
          perspectiveOrigin: "960px 540px",
        }}
      >
        {TILES.map((tile) => (
          <Tile key={tile.key} tile={tile} />
        ))}
      </View>

      <View
        key="sheet"
        className="absolute inset-0"
        style={{ opacity: sheet, backgroundImage: WARM }}
      >
        <Words italic color={INK}>
          over.
        </Words>
      </View>

      <View
        key="head"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: ink }}>N° 06 — FLIP</Text>
        <Text style={{ ...LABEL, color: ink }}>144 TILES · THREE PICTURES</Text>
      </View>

      <View
        key="foot"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, bottom: 64, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: ink }}>ONE WALL, TURNED TWICE</Text>
        <Text style={{ ...LABEL, color: ink }}>CSS 3D · NO MESHES</Text>
      </View>
    </View>
  );
}
