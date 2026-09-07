export const component = "particle-data-stream";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

export default function ParticleDataStream(ctx) {
  const t = ctx.hold.progress;
  const reveal = interpolate(t, [0, 0.14], [0, 1], { easing: "easeOut" });
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#020617" }}>
      <View key="field-glow" className="absolute" style={{ left: 350, top: 60, width: 580, height: 580, borderRadius: 290, backgroundColor: "#0e7490", opacity: 0.18, filter: "blur(110px)" }} />
      <GeometryBatch key="stream" geometry="circle"
        positions={particles(ctx.localFrame, ctx.fps, {
          seed: 20260810,
          count: 12000,
          emitter: rect(605, 650, 70, 8),
          birth: { interval: 0.00025 },
          lifetime: 2.4,
          velocity: { x: [-230, 230], y: [-410, -150] },
          gravity: point(0, 145),
          loop: true,
        })}
        sizes={[1.5, 7]}
        fills={["#22d3ee", "#a78bfa"]}
        opacities={[0.95, 0]}
        style={{ position: "absolute", left: 0, top: 0, width: 1280, height: 720 }} />

      <View key="core" className="absolute" style={{ borderStyle: "solid", left: 570, top: 555, width: 140, height: 140, borderRadius: 70, backgroundColor: "#0f172add", borderWidth: 2, borderColor: "#67e8f9", opacity: reveal, filter: "drop-shadow(0px 0px 38px #0891b2)" }} />
      <Text key="count" className="absolute" style={{ left: 0, top: 70, width: 1280, textAlign: "center", fontSize: 72, color: "#f8fafc", opacity: reveal }}>12,000</Text>
      <Text key="title" className="absolute" style={{ left: 0, top: 153, width: 1280, textAlign: "center", fontSize: 22, letterSpacing: 6, color: "#67e8f9", opacity: reveal }}>DETERMINISTIC PARTICLE FIELD</Text>
      <Text key="meta" className="absolute" style={{ left: 0, top: 205, width: 1280, textAlign: "center", fontSize: 16, color: "#64748b", opacity: reveal }}>ONE SCENE NODE · ONE DISPLAY COMMAND · RANDOM-ACCESS SEEK</Text>
    </Scene>
  );
}
