export const component = "effect-layering-probe";

export const controls = defineControls({
  props: {
    label: string({ default: "CONTROL", required: false }),
  },
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const bars = defineRepeater({ count: 10, keyPrefix: "bar" });

export default function LayerProbe(ctx, props) {
  const t = ctx.hold.progress;
  const pulse = (sin(t * 12.566370614) + 1) * 0.5;
  const localBlur = 2 + pulse * 11;

  return (
    <Scene className="relative h-full w-full" style={{ borderStyle: "solid", backgroundColor: "#0b1020", borderRadius: 52, borderWidth: 2, borderColor: "#334155" }}>
      <View key="grid-a" className="absolute" style={{ left: "8%", top: "14%", width: "84%", height: 1, backgroundColor: "#334155" }} />
      <View key="grid-b" className="absolute" style={{ left: "8%", top: "78%", width: "84%", height: 1, backgroundColor: "#334155" }} />
      <Text key="label" className="absolute" style={{ left: "8%", top: "7%", width: "84%", fontSize: 38, letterSpacing: 5, color: "#cbd5e1", textAlign: "center" }}>{props.label}</Text>

      <View key="local-filter" className="absolute" style={{ borderStyle: "solid", left: "27%", top: "24%", width: "46%", height: "42%", borderRadius: 90, backgroundColor: "#7c3aed", borderWidth: 4, borderColor: "#e0f2fe", filter: `blur(${localBlur}px) hue-rotate(${pulse * 48}deg) saturate(1.5) drop-shadow(0px 0px 36px #22d3eeaa)`, transform: `scale(${0.93 + pulse * 0.1})` }}>
        <Text key="mark" className="absolute" style={{ left: 0, top: "16%", width: "100%", fontSize: 128, color: "#ffffff", textAlign: "center" }}>V</Text>
      </View>

      {bars.map((bar) => (
        <View key={bar.key} className="absolute" style={{ left: `${10 + bar.index * 8}%`, bottom: "11%", width: "5%", height: `${5 + noise2d(909, bar.index * 0.31, t * 4) * 9}%`, borderRadius: 8, backgroundColor: bar.index % 2 === 0 ? "#22d3ee" : "#a78bfa", opacity: 0.45 + (1 - bar.progress) * 0.5 }} />
      ))}

      <Text key="inside" className="absolute" style={{ left: "8%", bottom: "3%", width: "84%", fontSize: 21, letterSpacing: 3, color: "#64748b", textAlign: "center" }}>MOTION SCENE CONTENT</Text>
    </Scene>
  );
}
