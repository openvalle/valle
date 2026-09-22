export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

export default function ParticleDataStream(ctx) {
  const t = ctx.progress;
  const reveal = interpolate(t, [0, 0.14], [0, 1], { easing: "easeOut" });
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#020617" }}>
      <View key="field-glow" className="absolute" style={{ left: 525, top: 90, width: 870, height: 870, borderRadius: 435, backgroundColor: "#0e7490", opacity: 0.18, filter: "blur(165px)" }} />
      <GeometryBatch key="stream" geometry="circle"
        positions={particles(ctx.localFrame, ctx.fps, {
          seed: 20260810,
          count: 12000,
          emitter: rect(907.5, 975, 105, 12),
          birth: { interval: 0.00025 },
          lifetime: 2.4,
          velocity: { x: [-230, 230], y: [-410, -150] },
          gravity: point(0, 217.5),
          loop: true,
        })}
        sizes={[1.5, 7]}
        fills={["#22d3ee", "#a78bfa"]}
        opacities={[0.95, 0]}
        style={{ position: "absolute", left: 0, top: 0, width: 1920, height: 1080 }} />

      <View key="core" className="absolute" style={{ borderStyle: "solid", left: 855, top: 832.5, width: 210, height: 210, borderRadius: 105, backgroundColor: "#0f172add", borderWidth: 3, borderColor: "#67e8f9", opacity: reveal, filter: "drop-shadow(0px 0px 57px #0891b2)" }} />
      <Text key="count" className="absolute" style={{ left: 0, top: 105, width: 1920, textAlign: "center", fontSize: 108, color: "#f8fafc", opacity: reveal }}>12,000</Text>
      <Text key="title" className="absolute" style={{ left: 0, top: 229.5, width: 1920, textAlign: "center", fontSize: 33, letterSpacing: 9, color: "#67e8f9", opacity: reveal }}>DETERMINISTIC PARTICLE FIELD</Text>
      <Text key="meta" className="absolute" style={{ left: 0, top: 307.5, width: 1920, textAlign: "center", fontSize: 24, color: "#64748b", opacity: reveal }}>ONE SCENE NODE · ONE DISPLAY COMMAND · RANDOM-ACCESS SEEK</Text>
    </Scene>
  );
}
