export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function Strobe(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <View key="bar" className="absolute"
        style={{ left: 0, top: 120, width: 20, height: 120,
          backgroundColor: "#ffffff", translate: point(fract(ctx.seconds) * 1200, 0) }} />
    </Scene>
  );
}
