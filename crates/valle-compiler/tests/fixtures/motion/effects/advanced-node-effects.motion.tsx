export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
const stripes = Array.from({ length: 11 }, (_, index) => ({ index }));

function Pattern(ctx, props) {
  return (
    <View style={{ position: "absolute", inset: 0, overflow: "hidden", borderRadius: 24 }}>
      {stripes.map(({ index }) => (
        <View
          key={`stripe-${index}`}
          style={{
            position: "absolute", left: index * 28 - 28, top: -30, width: 12, height: 220,
            rotate: "18deg", backgroundColor: index % 2 === 0 ? props.accent : "#ffffff",
            opacity: index % 2 === 0 ? 0.95 : 0.22,
          }}
        />
      ))}
      <View style={{ position: "absolute", left: 54, top: 34, width: 132, height: 64, borderRadius: 32, backgroundColor: "#050816", border: "2px solid #ffffff" }} />
    </View>
  );
}

export default function AdvancedNodeEffects(ctx) {
  const seconds = ctx.localFrame * ctx.fps.den / ctx.fps.num;
  const reveal = interpolate(ctx.progress, [0, 1], [0, 1], { easing: "easeOut" });
  const displacementScale = 18 + sin(seconds * 2.1) * 10;
  const travel = sin(seconds * 2.4);
  const travelY = cos(seconds * 1.7) * 18;
  const velocityX = cos(seconds * 2.4) * 230 * 2.4 / 60;
  const velocityY = -sin(seconds * 1.7) * 18 * 1.7 / 60;

  return (
    <Scene style={{ width: "100%", height: "100%", backgroundColor: "#050816", color: "#f8fafc" }}>
      <Text style={{ position: "absolute", left: 96, top: 66, fontSize: 26, letterSpacing: 4, color: "#22d3ee", opacity: reveal }}>ADVANCED NODE FILTERS</Text>
      <Text style={{ position: "absolute", left: 92, top: 120, width: 1720, fontSize: 68, fontWeight: 700, opacity: reveal }}>Noise bends a subtree. Velocity shapes its shutter.</Text>
      <Text style={{ position: "absolute", left: 98, top: 224, fontSize: 25, color: "#94a3b8", opacity: reveal }}>Bounded filters · deterministic seed · random-access frames · Native / CanvasKit</Text>

      <View style={{ position: "absolute", left: 96, top: 316, width: 830, height: 640, borderRadius: 30, border: "2px solid #1e293b", backgroundColor: "#080d1f" }}>
        <Text style={{ position: "absolute", left: 40, top: 32, fontSize: 28, letterSpacing: 2, color: "#a78bfa" }}>A / DISPLACEMENT FIELD</Text>
        <Text style={{ position: "absolute", left: 40, top: 78, fontSize: 22, color: "#94a3b8" }}>turbulence · seed 17 · 3 octaves</Text>
        <View style={{ position: "absolute", left: 42, top: 144, width: 746, height: 274, borderRadius: 28, backgroundColor: "#0f172a" }}>
          <View style={{ position: "absolute", left: 220, top: 62, width: 306, height: 150, displacement: displacement(17, point(0.007, 0.011), displacementScale, { octaves: 3, mode: "turbulence" }) }}>
            <Pattern accent="#d946ef" />
          </View>
        </View>
        <Text style={{ position: "absolute", left: 42, top: 448, width: 746, fontSize: 30 }}>Rasterized descendants bend together.</Text>
        <Text style={{ position: "absolute", left: 42, top: 500, width: 736, fontSize: 21, lineHeight: 1.35, color: "#94a3b8" }}>Text, paths and nested layout share one local field. Siblings outside the group stay stable.</Text>
        <View style={{ position: "absolute", left: 42, bottom: 34, width: 746, height: 5, backgroundColor: "#1e293b" }}>
          <View style={{ width: `${((displacementScale - 8) / 30) * 100}%`, height: "100%", backgroundColor: "#d946ef" }} />
        </View>
      </View>

      <View style={{ position: "absolute", left: 954, top: 316, width: 870, height: 640, borderRadius: 30, border: "2px solid #1e293b", backgroundColor: "#080d1f", overflow: "hidden" }}>
        <Text style={{ position: "absolute", left: 40, top: 32, fontSize: 28, letterSpacing: 2, color: "#22d3ee" }}>B / VELOCITY + SHUTTER</Text>
        <Text style={{ position: "absolute", left: 40, top: 78, fontSize: 22, color: "#94a3b8" }}>explicit px/frame velocity · shutter angle 210°</Text>
        <View style={{ position: "absolute", left: 42, top: 144, width: 786, height: 274, borderRadius: 137, backgroundColor: "#060a18", border: "2px solid #172036" }}>
          <View style={{ position: "absolute", left: `${50 + travel * 31}%`, top: `${48 + travelY / 15}%`, width: 250, height: 130, translate: "-50% -50%", motionBlur: motionBlur(point(velocityX, velocityY), 210) }}>
            <Pattern accent="#22d3ee" />
          </View>
          <View style={{ position: "absolute", left: "50%", top: 28, width: 2, height: 216, backgroundColor: "#1e293b" }} />
        </View>
        <Text style={{ position: "absolute", left: 42, top: 448, width: 786, fontSize: 29 }}>The blur follows the instantaneous velocity vector.</Text>
        <Text style={{ position: "absolute", left: 42, top: 506, width: 780, fontSize: 21, lineHeight: 1.4, color: "#94a3b8" }}>A spatial shutter approximation keeps each frame pure and available by random seek.</Text>
      </View>

      <Text style={{ position: "absolute", left: 98, bottom: 58, fontSize: 21, letterSpacing: 1.5, color: "#64748b" }}>CONTENT → CSS FILTER → NOISE DISPLACEMENT → VELOCITY BLUR → OPACITY / BLEND</Text>
    </Scene>
  );
}
