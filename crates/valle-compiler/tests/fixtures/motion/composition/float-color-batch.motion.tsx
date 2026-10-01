export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function FloatBatch(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <GeometryBatch key="swatch" geometry="rect" positions={[point(0, 0)]}
        sizes={point(640, 360)}
        fills={field({ from: "#2140ff", to: "#ffd000", progress: ctx.progress })}
        style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }} />
    </Scene>
  );
}
