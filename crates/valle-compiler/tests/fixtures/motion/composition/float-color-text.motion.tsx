export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function FloatText(ctx) {
  const color = interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
    { colorSpace: "oklch" });
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Text key="plain" className="absolute"
        style={{ left: 40, top: 45, fontSize: 80, color }}>FLOAT</Text>
      <Text key="units" className="absolute" split="char"
        style={{ left: 40, top: 200, fontSize: 80, color: "#ffffff" }}
        perUnit={{ color }}>UNITS</Text>
    </Scene>
  );
}
