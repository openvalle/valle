export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function EasingPresets(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <View key="back" className="absolute" style={{ top: 80, width: 40, height: 40, backgroundColor: "#ffffff",
        left: interpolate(ctx.seconds, [0, 1], [40, 440], { easing: "easeOutBack" }) }} />
      <View key="steps" className="absolute" style={{ top: 240, width: 40, height: 40, backgroundColor: "#ffffff",
        left: interpolate(ctx.seconds, [0, 1], [40, 440], { easing: "steps(4)" }) }} />
    </Scene>
  );
}
