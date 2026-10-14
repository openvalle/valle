export const composition = { width: 640, height: 360, fps: 60, duration: 10 };

// The glow input grows every frame, so every frame asks for a larger working surface.
export default function GrowGlow(ctx) {
  const r = 8 + clamp(ctx.localFrame, 0, 90) * 1.5;
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <View key="dot" className="absolute" style={{ left: 320 - r, top: 180 - r, width: r * 2, height: r * 2,
        borderRadius: r, backgroundColor: "#ffffff", filter: "glow(24px 1.5 #22d3ee)" }} />
    </Scene>
  );
}
