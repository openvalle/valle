export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const A = path("M 40 40 L 140 40 L 140 60 L 65 60 L 65 100 L 140 100 L 140 120 L 40 120 Z");
const B = path("M 340 40 L 450 40 L 450 60 L 367.5 60 L 367.5 100 L 450 100 L 450 120 L 340 120 Z");
const SA = path("M 40 220 L 140 220 L 140 240 L 65 240 L 65 280 L 140 280 L 140 300 L 40 300 Z");
const SB = path("M 270 220 L 380 220 L 380 240 L 297.5 240 L 297.5 280 L 380 280 L 380 300 L 270 300 Z");
const SC = path("M 470 220 L 590 220 L 590 240 L 500 240 L 500 280 L 590 280 L 590 300 L 470 300 Z");
const pairAnchors = [
  [point(140, 40), point(450, 40)],
  [point(140, 120), point(450, 120)],
];
const sequenceAnchors = [
  [point(140, 220), point(380, 220), point(590, 220)],
  [point(140, 300), point(380, 300), point(590, 300)],
];
export default function AnchorMorph(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="pair" d={morph(A, B, ctx.progress, { anchors: pairAnchors })} fill="#ffffff" />
      <Path key="sequence" d={morphSequence([SA, SB, SC], [0, 0.5, 1], ctx.progress, { method: "arcLength", anchors: sequenceAnchors })} fill="#22d3ee" />
    </Scene>
  );
}
