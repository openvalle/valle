export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function MultiFrame(ctx) {
  return (
    <Scene className="relative h-full w-full">
      <View key="dot" className="absolute" style={{ left: 40 + ctx.localFrame * 6, top: 130,
        width: 100, height: 100, borderRadius: 50, backgroundColor: "#ff804080" }} />
      <View key="solid" className="absolute" style={{ left: 40, top: 20,
        width: 100, height: 50, backgroundColor: "#2140ff" }} />
    </Scene>
  );
}
