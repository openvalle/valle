export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function ShutterSampling(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Shutter key="shutter" samples={8} angle={180}>
        <View key="blade" className="absolute" style={{ left: 120, top: 174, width: 400, height: 12,
          backgroundColor: "#ffffff", transform: `rotate(${clamp(ctx.seconds, 0, 1) * 180}deg)` }} />
      </Shutter>
    </Scene>
  );
}
