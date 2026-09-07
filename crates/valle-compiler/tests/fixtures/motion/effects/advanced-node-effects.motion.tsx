export const component = "advanced-node-effects";
const stripes = Array.from({ length: 11 }, (_, index) => ({ index }));

function Pattern(ctx, props) {
  return (
    <View style={{ position: "absolute", inset: 0, overflow: "hidden", borderRadius: 28 }}>
      {stripes.map(({ index }) => (
        <View
          key={`stripe-${index}`}
          style={{
            position: "absolute", left: index * 42 - 40, top: -50, width: 18, height: 330,
            rotate: "18deg", backgroundColor: index % 2 === 0 ? props.accent : "#ffffff",
            opacity: index % 2 === 0 ? 0.95 : 0.22,
          }}
        />
      ))}
      <View style={{ position: "absolute", left: 78, top: 58, width: 190, height: 86, borderRadius: 43, backgroundColor: "#050816", border: "3px solid #ffffff" }} />
    </View>
  );
}

export default function AdvancedNodeEffects(ctx) {
  const seconds = ctx.localFrame * ctx.fps.den / ctx.fps.num;
  const reveal = interpolate(ctx.enter.progress, [0, 1], [0, 1], { easing: "easeOut" });
  const displacementScale = 22 + sin(seconds * 2.1) * 12;
  const travel = sin(seconds * 2.4);
  const travelY = cos(seconds * 1.7) * 26;
  const velocityX = cos(seconds * 2.4) * 230 * 2.4 / 60;
  const velocityY = -sin(seconds * 1.7) * 26 * 1.7 / 60;

  return (
    <Scene style={{ width: "100%", height: "100%", backgroundColor: "#050816", color: "#f8fafc" }}>
      <Text style={{ position: "absolute", left: 108, top: 96, fontSize: 24, letterSpacing: 4, color: "#22d3ee", opacity: reveal }}>ADVANCED NODE FILTERS</Text>
      <Text style={{ position: "absolute", left: 108, top: 145, width: 2500, fontSize: 64, fontWeight: 700, opacity: reveal }}>Noise can bend a subtree. Velocity can shape its shutter.</Text>
      <Text style={{ position: "absolute", left: 112, top: 232, fontSize: 22, color: "#94a3b8", opacity: reveal }}>One bounded BeginFilter chain · deterministic seed · random-access frames · Native / CanvasKit shared wire</Text>

      <View style={{ position: "absolute", left: 112, top: 390, width: 1100, height: 1380, borderRadius: 48, border: "2px solid #1e293b", backgroundColor: "#080d1f" }}>
        <Text style={{ position: "absolute", left: 54, top: 48, fontSize: 22, letterSpacing: 3, color: "#a78bfa" }}>A / DISPLACEMENT FIELD</Text>
        <Text style={{ position: "absolute", left: 54, top: 90, fontSize: 18, color: "#64748b" }}>turbulence · seed 17 · 3 octaves</Text>
        <View style={{ position: "absolute", left: 92, top: 310, width: 900, height: 520, borderRadius: 56, backgroundColor: "#0f172a" }}>
          <View style={{ position: "absolute", left: 275, top: 145, width: 350, height: 210, displacement: displacement(17, point(0.007, 0.011), displacementScale, { octaves: 3, mode: "turbulence" }) }}>
            <Pattern accent="#d946ef" />
          </View>
        </View>
        <Text style={{ position: "absolute", left: 92, top: 890, fontSize: 26 }}>Rasterized descendants bend together.</Text>
        <Text style={{ position: "absolute", left: 92, top: 940, width: 850, fontSize: 18, lineHeight: 1.6, color: "#94a3b8" }}>Text, paths, images and nested layout can share one local field. Siblings outside the group remain pixel-stable.</Text>
        <View style={{ position: "absolute", left: 92, bottom: 96, width: 900, height: 5, backgroundColor: "#1e293b" }}>
          <View style={{ width: `${((displacementScale - 10) / 36) * 100}%`, height: "100%", backgroundColor: "#d946ef" }} />
        </View>
      </View>

      <View style={{ position: "absolute", right: 112, top: 390, width: 2400, height: 1380, borderRadius: 48, border: "2px solid #1e293b", backgroundColor: "#080d1f", overflow: "hidden" }}>
        <Text style={{ position: "absolute", left: 54, top: 48, fontSize: 22, letterSpacing: 3, color: "#22d3ee" }}>B / VELOCITY + SHUTTER</Text>
        <Text style={{ position: "absolute", left: 54, top: 90, fontSize: 18, color: "#64748b" }}>explicit px/frame velocity · shutter angle 210°</Text>
        <View style={{ position: "absolute", left: 100, top: 250, width: 2200, height: 700, borderRadius: 350, backgroundColor: "#060a18", border: "2px solid #172036" }}>
          <View style={{ position: "absolute", left: `${50 + travel * 31}%`, top: `${44 + travelY / 20}%`, width: 410, height: 210, translate: "-50% -50%", motionBlur: motionBlur(point(velocityX, velocityY), 210) }}>
            <Pattern accent="#22d3ee" />
          </View>
          <View style={{ position: "absolute", left: "50%", top: 54, width: 2, height: 590, backgroundColor: "#1e293b" }} />
        </View>
        <Text style={{ position: "absolute", left: 104, top: 1020, fontSize: 28 }}>The blur follows the instantaneous velocity vector.</Text>
        <Text style={{ position: "absolute", left: 104, top: 1075, width: 2050, fontSize: 19, lineHeight: 1.6, color: "#94a3b8" }}>This first contract is a spatial shutter approximation: frame-pure and random-access. Temporal occlusion and rotating-subtree multi-sampling remain a different, more expensive future tier.</Text>
      </View>

      <Text style={{ position: "absolute", left: 112, bottom: 96, fontSize: 18, letterSpacing: 2, color: "#64748b" }}>CONTENT → CSS FILTER → NOISE DISPLACEMENT → VELOCITY BLUR → OPACITY / BLEND</Text>
    </Scene>
  );
}
