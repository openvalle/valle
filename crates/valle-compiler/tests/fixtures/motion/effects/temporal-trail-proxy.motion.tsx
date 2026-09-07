export const component = "temporal-trail-proxy";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const samples = defineRepeater({ count: 30, keyPrefix: "sample" });
const ROUTE = path("M 178 378 C 160 170 418 92 622 242 C 820 388 1106 260 1102 492 C 1098 666 802 630 630 500 C 456 370 196 610 178 378 Z");

export default function TemporalTrailProxy(ctx) {
  const t = ctx.hold.progress;
  const head = t * 1.22;
  const reveal = interpolate(t, [0, 0.16], [0, 1], { easing: "easeOut" });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#030712" }}>
      <View key="blue-glow" className="absolute" style={{ left: 210, top: 165, width: 460, height: 380, borderRadius: 220, backgroundColor: "#2563eb", opacity: 0.15, filter: "blur(110px)" }} />
      <View key="pink-glow" className="absolute" style={{ left: 670, top: 210, width: 390, height: 340, borderRadius: 190, backgroundColor: "#db2777", opacity: 0.13, filter: "blur(100px)" }} />

      <Text key="kicker" className="absolute" style={{ left: 62, top: 44, fontSize: 15, letterSpacing: 4, color: "#f472b6", opacity: reveal }}>TEMPORAL APPEARANCE</Text>
      <Text key="title" className="absolute" style={{ left: 58, top: 79, width: 1120, fontSize: 50, color: "#f8fafc", opacity: reveal }}>A useful trail is not true motion blur.</Text>
      <Text key="subtitle" className="absolute" style={{ left: 62, top: 142, width: 900, fontSize: 18, color: "#94a3b8", opacity: reveal }}>Thirty frame-pure samples approximate a shutter trail along one path.</Text>

      <Path key="route-wide" d={ROUTE} fill="none" stroke="#172554b3" strokeWidth="24" />
      <Path key="route-fine" d={ROUTE} fill="none" stroke="#334155e6" strokeWidth="2" strokeDasharray="8 14" />

      {samples.map((sample) => (
        <View key={sample.key} className="absolute" style={{ width: 34 - sample.progress * 19, height: 14 - sample.progress * 7, borderRadius: 18, backgroundColor: sample.index % 3 === 0 ? "#f8fafc" : sample.index % 3 === 1 ? "#67e8f9" : "#f472b6", opacity: (1 - sample.progress) * 0.48, filter: `blur(${0.4 + sample.progress * 5.2}px) drop-shadow(0px 0px 9px #67e8f9)`, motionPath: follow(ROUTE, trail(head, sample.index, { gap: 0.012, mode: "wrap" }), { rotate: "auto" }) }} />
      ))}

      <View key="head" className="absolute flex items-center justify-center" style={{ borderStyle: "solid", width: 76, height: 34, borderRadius: 18, backgroundColor: "#f8fafc", borderWidth: 2, borderColor: "#67e8f9", filter: "drop-shadow(0px 0px 18px #22d3ee)", motionPath: follow(ROUTE, head, { rotate: "auto" }) }}>
        <View key="head-core" style={{ width: 28, height: 6, borderRadius: 3, backgroundColor: "#0f172a" }} />
      </View>

      <View key="verdict" className="absolute" style={{ borderStyle: "solid", left: 62, top: 604, width: 1156, height: 72, borderTopWidth: 1, borderColor: "#334155" }}>
        <Text key="verdict-now" className="absolute" style={{ left: 0, top: 19, fontSize: 14, letterSpacing: 2, color: "#67e8f9" }}>NOW · REPEATER + TRAIL</Text>
        <Text key="verdict-gap" className="absolute" style={{ right: 0, top: 19, fontSize: 14, letterSpacing: 2, color: "#f472b6" }}>GAP · VELOCITY-AWARE NODE MOTION BLUR</Text>
      </View>
    </Scene>
  );
}
