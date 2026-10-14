export const composition = { width: 640, height: 360, fps: 60, duration: 10 };

// A glow under a clip: the bar extends far beyond the window, which shows only a small part.
export default function ClipGlow(ctx) {
  const x = -900 + clamp(ctx.localFrame, 0, 90) * 4;
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <View key="window" className="absolute" style={{ left: 200, top: 100, width: 240, height: 160, overflow: "hidden" }}>
        <View key="glow" className="absolute" style={{ left: 0, top: 0, width: 240, height: 160, filter: "glow(32px 1.2 #f472b6)" }}>
          <View key="bar" className="absolute" style={{ left: x, top: 60, width: 1200, height: 40, backgroundColor: "#ffffff" }} />
        </View>
      </View>
    </Scene>
  );
}
