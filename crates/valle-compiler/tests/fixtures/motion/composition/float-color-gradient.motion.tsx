export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function FloatGradient(ctx) {
  const color = interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
    { colorSpace: "oklch" });
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="gradient" d="M 0 0 L 640 0 L 640 360 L 0 360 Z"
        fill={linearGradient(point(0, 0), point(640, 0), [
          gradientStop(0, color), gradientStop(1, color),
        ])} />
    </Scene>
  );
}
