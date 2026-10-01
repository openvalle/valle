export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function SmoothWeight(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Text key="t" className="absolute" style={{ left: 60, top: 130, fontSize: 80, color: "#ffffff",
        fontWeight: 100 + ctx.progress * 800 }}>WEIGHT</Text>
    </Scene>
  );
}
