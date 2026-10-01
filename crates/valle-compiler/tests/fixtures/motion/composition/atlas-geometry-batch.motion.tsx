export const composition = { width: 320, height: 160, fps: 30, duration: 2 };
export const controls = { assets: { sprites: asset({ kind: "image", required: true }) } };
const TILE = atlasRegion("asset://sprites", rect(0, 0, 0.5, 1));

export default function AtlasBatch(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <GeometryBatch key="tiles" geometry={TILE}
        positions={[point(24, 40), point(184, 40)]}
        sizes={[point(80, 80), point(80, 80)]}
        fills={["#ffffff", "#80c0ff"]}
        rotations={[0, 36]} opacities={[1, 0.7]}
        style={{ position: "absolute", left: 0, top: 0, width: 320, height: 160 }} />
    </Scene>
  );
}
