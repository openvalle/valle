export const composition = { width: 640, height: 360, fps: 30, duration: 1 };
export default function AlphaExport(ctx) {
  return (
    <Scene className="relative h-full w-full">
      <View key="dot" className="absolute" style={{ left: 270, top: 130, width: 100, height: 100, borderRadius: 50,
        backgroundColor: "#ffffff" }} />
    </Scene>
  );
}
