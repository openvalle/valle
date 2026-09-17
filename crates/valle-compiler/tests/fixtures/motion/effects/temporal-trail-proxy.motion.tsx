export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "temporal-trail-proxy";

export const controls = defineControls({
  timing: { enterDuration: 0, exitDuration: 0 },
});

const samples = defineRepeater({ count: 30, keyPrefix: "sample" });
const ROUTE = path("M 267 567 C 240 255 627 138 933 363 C 1230 582 1659 390 1653 738 C 1647 999 1203 945 945 750 C 684 555 294 915 267 567 Z");

export default function TemporalTrailProxy(ctx) {
  const t = ctx.hold.progress;
  const head = t * 1.22;
  const reveal = interpolate(t, [0, 0.16], [0, 1], { easing: "easeOut" });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#030712" }}>
      <View key="blue-glow" className="absolute" style={{ left: 315, top: 247.5, width: 690, height: 570, borderRadius: 330, backgroundColor: "#2563eb", opacity: 0.15, filter: "blur(165px)" }} />
      <View key="pink-glow" className="absolute" style={{ left: 1005, top: 315, width: 585, height: 510, borderRadius: 285, backgroundColor: "#db2777", opacity: 0.13, filter: "blur(150px)" }} />

      <Text key="kicker" className="absolute" style={{ left: 93, top: 66, fontSize: 22.5, letterSpacing: 6, color: "#f472b6", opacity: reveal }}>TEMPORAL APPEARANCE</Text>
      <Text key="title" className="absolute" style={{ left: 87, top: 118.5, width: 1680, fontSize: 75, color: "#f8fafc", opacity: reveal }}>A useful trail is not true motion blur.</Text>
      <Text key="subtitle" className="absolute" style={{ left: 93, top: 213, width: 1350, fontSize: 27, color: "#94a3b8", opacity: reveal }}>Thirty frame-pure samples approximate a shutter trail along one path.</Text>

      <Path key="route-wide" d={ROUTE} fill="none" stroke="#172554b3" strokeWidth="36" />
      <Path key="route-fine" d={ROUTE} fill="none" stroke="#334155e6" strokeWidth="3" strokeDasharray="8 14" />

      {samples.map((sample) => (
        <View key={sample.key} className="absolute" style={{ width: 51 - sample.progress * 28.5, height: 21 - sample.progress * 10.5, borderRadius: 27, backgroundColor: sample.index % 3 === 0 ? "#f8fafc" : sample.index % 3 === 1 ? "#67e8f9" : "#f472b6", opacity: (1 - sample.progress) * 0.48, filter: `blur(${0.4 + sample.progress * 5.2}px) drop-shadow(0px 0px 13.5px #67e8f9)`, motionPath: follow(ROUTE, trail(head, sample.index, { gap: 0.012, mode: "wrap" }), { rotate: "auto" }) }} />
      ))}

      <View key="head" className="absolute flex items-center justify-center" style={{ borderStyle: "solid", width: 114, height: 51, borderRadius: 27, backgroundColor: "#f8fafc", borderWidth: 3, borderColor: "#67e8f9", filter: "drop-shadow(0px 0px 27px #22d3ee)", motionPath: follow(ROUTE, head, { rotate: "auto" }) }}>
        <View key="head-core" style={{ width: 42, height: 9, borderRadius: 4.5, backgroundColor: "#0f172a" }} />
      </View>

      <View key="verdict" className="absolute" style={{ borderStyle: "solid", left: 93, top: 906, width: 1734, height: 108, borderTopWidth: 1.5, borderColor: "#334155" }}>
        <Text key="verdict-now" className="absolute" style={{ left: 0, top: 28.5, fontSize: 21, letterSpacing: 3, color: "#67e8f9" }}>NOW · REPEATER + TRAIL</Text>
        <Text key="verdict-gap" className="absolute" style={{ right: 0, top: 28.5, fontSize: 21, letterSpacing: 3, color: "#f472b6" }}>GAP · VELOCITY-AWARE NODE MOTION BLUR</Text>
      </View>
    </Scene>
  );
}
