export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const POINTS = Array.from({ length: 5 }, (_, i) => point(120 + i * 100, 180));
const DELAYS = Array.from({ length: 5 }, (_, i) => Math.abs(i - 2) * 0.2);

export default function DistanceStagger(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <GeometryBatch geometry="rect" positions={POINTS} sizes={point(48, 48)}
        fills="#ffffff"
        opacities={field({ from: 0, to: 1, progress: ctx.progress, stagger: DELAYS })}
        style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }} />
    </Scene>
  );
}
