export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Selector(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Text key="t" split="char" className="absolute" style={{ left: 40, top: 130, fontSize: 90, fontWeight: 800,
        color: "#ffffff" }}
        perUnit={{
          opacity: rangeSelector({ start: 0, end: ctx.progress, softness: 0.15, shape: "ramp" }),
          blur: 8 * (1 - rangeSelector({ start: 0, end: ctx.progress, softness: 0.15, shape: "ramp" })),
        }}>SELECTOR</Text>
    </Scene>
  );
}
