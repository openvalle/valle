export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function MotionBlurAuto(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <View key="bar" className="absolute"
        style={{ left: 40, top: 120, width: 40, height: 120,
          backgroundColor: "#ffffff",
          translate: point(clamp(ctx.seconds, 0, 0.5) * 600, 0),
          motionBlur: "auto" }} />
    </Scene>
  );
}
