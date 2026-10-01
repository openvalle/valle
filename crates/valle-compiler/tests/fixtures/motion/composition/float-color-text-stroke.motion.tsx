export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function FloatTextStroke(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Text key="outline" style={{ position: "absolute", left: 90, top: 100,
        fontSize: 110, color: "#00000000", WebkitTextStrokeWidth: 6,
        WebkitTextStrokeColor: interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
          { colorSpace: "oklch" }) }}>VALLE</Text>
    </Scene>
  );
}
