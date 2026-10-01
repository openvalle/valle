export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function OklchInterpolate(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <View key="swatch" className="absolute"
        style={{ left: 0, top: 0, width: 640, height: 360,
          backgroundColor: interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
            { colorSpace: "oklch", hue: "shorter" }) }} />
    </Scene>
  );
}
