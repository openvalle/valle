// Editorial 3D cards: three story cards rise in perspective, flip in turn, fan out and sway.
// Shows CSS 3D (perspective, perspectiveOrigin, transformStyle "preserve-3d",
// backfaceVisibility, translate3d/rotateX/rotateY lists), a mapped card component, springs,
// a path drawn with trimEnd, and four small effects: a ShaderLayer foil on one card face,
// a Scene3D badge, a Glass caption and a backdropFilter masthead.
// Render: valle motion render examples/09-cards-3d.motion.tsx -o 09-cards-3d.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

const PAPER = "#FFF8F0";
const INK = "#2D3047";
const BLUE = "#3E92CC";
const RED = "#E84855";

const CARD_W = 420;
const CARD_H = 580;
const CENTER = { x: 960, y: 570 };
const SWAY_START = 2.6;

const CARDS = [
  { id: "motion", number: "01", title: "Motion", line: "Every frame, a function of time", accent: BLUE },
  { id: "type", number: "02", title: "Type", line: "Letters that move with intent", accent: RED },
  { id: "color", number: "03", title: "Color", line: "Bold palettes, quiet balance", accent: INK },
];

function starPath(outer: number, inner: number) {
  const corners = Array.from({ length: 10 }, (_, i) => {
    const r = i % 2 === 0 ? outer : inner;
    const a = -Math.PI / 2 + (i * Math.PI) / 5;
    return `${i === 0 ? "M" : "L"} ${(r * Math.cos(a)).toFixed(2)} ${(r * Math.sin(a)).toFixed(2)}`;
  });
  return path(`${corners.join(" ")} Z`);
}
const BADGE = roundCorners(starPath(112, 54), 12);
// The Motion card's art: an ease-in-out curve on two axes.
const AXES = path("M 56 44 L 56 236 L 316 236");
const CURVE = path("M 56 236 C 176 236 196 64 316 64");
const RINGS = [240, 400, 560, 720, 880, 1040];

// Foil: a faint rainbow over the face and a brighter band that the tilt slides across. The layer
// sits inside a paper face, which draws the card's edge, and the foil fades out before its own.
const FOIL_INSET = 12;
const FOIL = `
uniform float sweep;
uniform float hue;

float3 spectrum(float h) {
  return clamp(abs(fract(h + float3(0.0, 0.6667, 0.3333)) * 6.0 - 3.0) - 1.0, 0.0, 1.0);
}

float4 valle_main(float2 uv) {
  float4 card = sampleContent(uv);
  float d = uv.x * 0.7 + uv.y * 0.45;
  float band = exp(-(d - sweep) * (d - sweep) * 24.0);
  float3 foil = spectrum(d * 1.6 + hue) * 0.7 + 0.3;
  float edge = smoothstep(0.0, 0.08, uv.x) * smoothstep(0.0, 0.08, 1.0 - uv.x)
    * smoothstep(0.0, 0.06, uv.y) * smoothstep(0.0, 0.06, 1.0 - uv.y);
  return float4(mix(card.rgb, foil, (0.05 + 0.3 * band) * edge), card.a);
}`;

// The deck's sway, shared by the deck, the floor shadow and the foil.
function swayRamp(seconds: number) {
  return interpolate(seconds, [SWAY_START, SWAY_START + 1], [0, 1], { easing: "easeInOutSine" });
}
function swayTurn(seconds: number) {
  return Math.sin((seconds - SWAY_START) * 1.9) * swayRamp(seconds);
}
function swayTilt(seconds: number) {
  return Math.sin((seconds - SWAY_START) * 1.3) * swayRamp(seconds);
}

function Art({ card }) {
  const { seconds } = useCurrentTime();
  if (card.id === "motion") {
    const draw = interpolate(seconds, [1.55, 2.3], [0, 1], { easing: "easeInOutCubic" });
    const arrive = spring(seconds - 2.2, { duration: 0.45, bounce: 0.4 });
    return (
      <View key="art" className="relative" style={{ width: 360, height: 280, borderRadius: 18,
        backgroundColor: card.accent, overflow: "hidden" }}>
        <Path key="axes" d={AXES} fill="none" stroke="#FFF8F066" strokeWidth={4} strokeLinecap="round"
          strokeLinejoin="round" style={{ position: "absolute", left: 0, top: 0 }} />
        <Path key="curve" d={CURVE} fill="none" stroke={PAPER} strokeWidth={9} strokeLinecap="round"
          trimEnd={draw} style={{ position: "absolute", left: 0, top: 0 }} />
        <Circle key="start" cx={56} cy={236} r={9} fill={PAPER} />
        <View key="end" className="absolute rounded-full" style={{ left: 302, top: 50, width: 28, height: 28,
          backgroundColor: PAPER, scale: arrive }} />
      </View>
    );
  }
  if (card.id === "type") {
    return (
      <View key="art" className="flex items-center justify-center" style={{ width: 360, height: 280,
        borderRadius: 18, backgroundColor: card.accent }}>
        <Text key="glyphs" style={{ fontFamily: "serif", fontWeight: 700, fontSize: 170, lineHeight: 1,
          color: PAPER }}>Aa</Text>
      </View>
    );
  }
  return (
    <View key="art" className="relative" style={{ width: 360, height: 280, borderRadius: 18,
      backgroundColor: card.accent, overflow: "hidden" }}>
      <View key="disc-blue" className="absolute rounded-full" style={{ left: 78, top: 34, width: 140, height: 140, backgroundColor: "#3E92CCE6" }} />
      <View key="disc-red" className="absolute rounded-full" style={{ left: 142, top: 34, width: 140, height: 140, backgroundColor: "#E84855D9" }} />
      <View key="disc-sun" className="absolute rounded-full" style={{ left: 110, top: 100, width: 140, height: 140, backgroundColor: "#FFB347D9" }} />
    </View>
  );
}

function Front({ card }) {
  return (
    <View key="content" className="absolute flex flex-col items-center"
      style={{ left: 0, top: 0, width: CARD_W, height: CARD_H, paddingTop: 30, borderRadius: 26,
        backgroundColor: PAPER }}>
      <Art card={card} />
      <Text key="title" style={{ marginTop: 30, fontFamily: "serif", fontWeight: 700, fontSize: 64,
        lineHeight: 1, color: INK }}>{card.title}</Text>
      <Text key="line" style={{ marginTop: 30, fontSize: 24, lineHeight: 1.3, color: "#2D3047B3" }}>
        {card.line}
      </Text>
      <View key="rule" style={{ marginTop: 24, width: 300, height: 2, backgroundColor: "#2D30471F" }} />
      <Text key="meta" style={{ marginTop: 18, fontSize: 18, fontWeight: 700, letterSpacing: 3,
        color: card.accent }}>{`STORY ${card.number} · 6 MIN READ`}</Text>
    </View>
  );
}

function Card({ card, index }) {
  const { seconds } = useCurrentTime();
  const side = index - 1;
  const t = seconds - 0.1 - index * 0.07;
  const rise = spring(t, { duration: 0.8, bounce: 0.25 });
  const appear = interpolate(t, [0, 0.25], [0, 1], { easing: "easeOutCubic" });
  const flip = interpolate(seconds - index * 0.2, [1.15, 1.85], [0, 1], { easing: "easeInOutCubic" });
  const fan = interpolate(seconds, [2.55, 3.35], [0, 1], { easing: "easeInOutCubic" });
  const x = side * (500 - 30 * rise + 30 * fan);
  const y = (1 - rise) * 260 + Math.abs(side) * 30 * fan;
  const z = -(1 - rise) * 900 + Math.sin(flip * TAU / 2) * 120 + (side === 0 ? 60 : -40) * fan;
  const turnZ = side * 5 * fan - side * 8 * (1 - rise);
  const tiltX = 60 * (1 - rise);
  const turnY = 180 * (1 - flip) - side * 12 * fan;
  // The foil band crosses the middle face after its flip, then rides the sway.
  const sweep = interpolate(seconds, [1.75, 2.55], [-0.35, 0.55], { easing: "easeOutCubic" }) + 0.35 * swayTurn(seconds);
  return (
    <View key="card" className="absolute"
      style={{ left: CENTER.x - CARD_W / 2, top: CENTER.y - CARD_H / 2, width: CARD_W, height: CARD_H,
        transformStyle: "preserve-3d",
        transform: `translate3d(${x}px, ${y}px, ${z}px) rotateZ(${turnZ}deg) rotateX(${tiltX}deg) rotateY(${turnY}deg)` }}>
      <View key="back" className="absolute flex items-center justify-center"
        style={{ left: 0, top: 0, width: CARD_W, height: CARD_H, borderRadius: 26, backgroundColor: INK,
          backfaceVisibility: "hidden", transform: "rotateY(180deg)", opacity: appear }}>
        <View key="frame" className="absolute" style={{ left: 18, top: 18, width: CARD_W - 36, height: CARD_H - 36,
          borderRadius: 16, border: "2px solid #FFF8F038" }} />
        <View key="ring" className="absolute rounded-full" style={{ left: CARD_W / 2 - 120, top: CARD_H / 2 - 120,
          width: 240, height: 240, border: "4px solid #E84855" }} />
        <Text key="mark" style={{ fontFamily: "serif", fontWeight: 700, fontSize: 170, lineHeight: 1,
          color: PAPER }}>V</Text>
      </View>
      <View key="front" className="absolute"
        style={{ left: 0, top: 0, width: CARD_W, height: CARD_H, backfaceVisibility: "hidden" }}>
        {card.id === "type" ? (
          <View key="paper" className="absolute"
            style={{ left: 0, top: 0, width: CARD_W, height: CARD_H, borderRadius: 26, backgroundColor: PAPER }}>
            <ShaderLayer key="foil" code={FOIL} uniforms={{ sweep, hue: seconds * 0.15 + 0.3 * swayTilt(seconds) }}
              style={{ position: "absolute", left: FOIL_INSET, top: FOIL_INSET,
                width: CARD_W - 2 * FOIL_INSET, height: CARD_H - 2 * FOIL_INSET }}>
              <View key="shift" className="absolute"
                style={{ left: -FOIL_INSET, top: -FOIL_INSET, width: CARD_W, height: CARD_H }}>
                <Front card={card} />
              </View>
            </ShaderLayer>
          </View>
        ) : (
          <Front card={card} />
        )}
      </View>
    </View>
  );
}

function Badge() {
  const { seconds } = useCurrentTime();
  const pop = spring(seconds - 2.15, { duration: 0.7, bounce: 0.35 });
  const spin = interpolate(seconds, [2.15, 3.4], [-360, 0], { easing: "easeOutCubic" })
    + 24 * Math.sin((seconds - 3.4) * 1.7) * interpolate(seconds, [3.4, 4.2], [0, 1], { easing: "easeInOutSine" });
  return (
    <Scene3D key="badge" style={{ position: "absolute", left: CENTER.x - 150, top: 0, width: 300, height: 300 }}
      camera={{ position: [0, 0, 700], target: [0, 0, 0], fov: 32 }}>
      <AmbientLight intensity={0.5} />
      <HemisphereLight skyColor="#FFF8F0" groundColor="#FF8C42" intensity={0.7} />
      <DirectionalLight direction={[-0.45, 0.75, 0.8]} intensity={2.4} />
      <Mesh key="star" geometry={extrude(BADGE, { depth: 36, bevel: 7 })}
        material={{ type: "pbr", color: RED, metallic: 0.2, roughness: 0.32 }}
        position={[0, 10 * Math.sin((seconds - 2.15) * 2.4), 0]}
        rotation={[8, spin, 0]} scale={[0.001 + pop, 0.001 + pop, 0.001 + pop]} />
    </Scene3D>
  );
}

export default function CardsDeck() {
  const { seconds, progress } = useCurrentTime();
  const mast = interpolate(seconds, [0.2, 0.8], [0, 1], { easing: "easeOutCubic" });
  const caption = interpolate(seconds, [3.1, 3.7], [0, 1], { easing: "easeOutCubic" });
  return (
    <Scene className="relative h-full w-full"
      style={{ backgroundImage: "radial-gradient(ellipse at 50% 34%, #FFA766 0%, #FF8C42 55%, #F4762F 100%)" }}>
      {RINGS.map((r) => (
        <Circle key={`ring-${r}`} cx={CENTER.x} cy={CENTER.y - 40} r={r * (0.96 + 0.04 * progress)}
          fill="none" stroke="#FFF8F040" strokeWidth={2.5} />
      ))}

      <View key="masthead" className="absolute flex items-center"
        style={{ left: 72, top: 56, height: 60, paddingLeft: 24, paddingRight: 28, gap: 14, borderRadius: 30,
          backgroundColor: "#FFF8F030", border: "1.5px solid #FFF8F080", backdropFilter: "blur(4px) saturate(1.2)",
          opacity: mast, translate: point(0, (1 - mast) * -18) }}>
        <View key="dot" style={{ width: 12, height: 12, borderRadius: 6, backgroundColor: RED }} />
        <Text key="mast-text" style={{ fontSize: 22, fontWeight: 700, letterSpacing: 3, color: INK }}>
          VALLE QUARTERLY · Nº 07
        </Text>
      </View>
      <Text key="season" className="absolute"
        style={{ right: 76, top: 74, fontSize: 22, fontWeight: 700, letterSpacing: 3, color: "#2D3047A6", opacity: mast }}>
        SPRING 2026
      </Text>

      <View key="floor" className="absolute"
        style={{ left: 260, top: 790, width: 1400, height: 160, translate: point(-40 * swayTurn(seconds), 0),
          opacity: interpolate(seconds, [0.1, 0.9], [0, 1]),
          backgroundImage: "radial-gradient(ellipse at center, #9A3B0F4D 0%, #9A3B0F00 70%)" }} />

      <View key="stage" className="absolute"
        style={{ left: 0, top: 0, width: 1920, height: 1080, perspective: 1600, perspectiveOrigin: "960px 480px" }}>
        <View key="deck" className="absolute"
          style={{ left: 0, top: 0, width: 1920, height: 1080, transformStyle: "preserve-3d",
            transformOrigin: `${CENTER.x}px ${CENTER.y}px`,
            transform: `rotateX(${4 - 4 * swayRamp(seconds) + 3 * swayTilt(seconds)}deg) rotateY(${7 * swayTurn(seconds)}deg)` }}>
          {CARDS.map((card, index) => (
            <Card key={card.id} card={card} index={index} />
          ))}
        </View>
      </View>

      <Badge key="badge" />

      <Glass key="caption" className="absolute flex items-center justify-center"
        material={{ bevel: 24, refraction: 14, dispersion: 0.35, frost: 3, tint: "#FFF8F033" }}
        light={{ intensity: 0.4 }} presence={caption}
        style={{ left: 570, top: 908, width: 780, height: 96, borderRadius: 48,
          translate: point(0, (1 - caption) * 36) }}>
        <Text key="caption-text" style={{ fontSize: 29, fontWeight: 600, letterSpacing: 0.5,
          color: interpolate(caption, [0, 1], ["#2D304700", INK]) }}>
          Three stories on motion, type and color
        </Text>
      </Glass>
    </Scene>
  );
}
