export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const A = path("M 50 40 L 150 40 L 150 140 L 50 140 Z");
const B = path("M 430 30 L 560 85 L 440 150 Z");
const SA = path("M 40 230 L 180 230 L 180 330 L 40 330 Z");
const SB = path("M 490 205 L 565 280 L 490 335 L 415 280 Z");
const SC = arc(point(300, 280), 46, 0, TAU);
export default function Convex(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="pair" d={morph(A, B, ctx.progress, { method: "convex" })} fill="#ffffff" />
      <Path key="sequence" d={morphSequence([SA, SB, SC], [0, 0.5, 1], ctx.progress, { method: "convex" })} fill="#22d3ee" />
    </Scene>
  );
}
