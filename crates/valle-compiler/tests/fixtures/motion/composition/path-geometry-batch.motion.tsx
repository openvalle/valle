export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const STAR = path("M 0 -30 L 7 -10 L 29 -9 L 12 4 L 18 25 L 0 13 L -18 25 L -12 4 L -29 -9 L -7 -10 Z");
export default function BatchV2(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <GeometryBatch key="stars" geometry={STAR} positions={[point(160, 180), point(480, 180)]}
        sizes={[1, 1]} fills="#ffd000" rotations={[0, 36]} opacities={[1, 1]}
        style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }} />
    </Scene>
  );
}
