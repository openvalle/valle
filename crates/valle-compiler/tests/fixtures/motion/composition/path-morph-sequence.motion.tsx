export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const STAR = path("M 320.0 50.0 L 352.3 135.5 L 443.6 139.8 L 372.3 197.0 L 396.4 285.2 L 320.0 235.0 L 243.6 285.2 L 267.7 197.0 L 196.4 139.8 L 287.7 135.5 Z");
const SQUARE = path("M 220 80 L 420 80 L 420 280 L 220 280 Z");
const CIRCLE = arc(point(320, 180), 120, 0, TAU);
export default function MorphSequence(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="shape" d={morphSequence([STAR, SQUARE, CIRCLE], [0, 0.5, 1], ctx.progress)} fill="#ffffff" />
    </Scene>
  );
}
