export const composition = { width: 160, height: 120, fps: 30, duration: 2 };
const A = path("M 12 8 L 60 8 L 60 48 L 36 30 L 12 48 Z");
const B = path("M 86 8 L 144 8 L 134 28 L 144 50 L 86 50 Z");
const SA = path("M 12 70 L 60 70 L 60 110 L 36 92 L 12 110 Z");
const SB = path("M 86 70 L 144 70 L 134 90 L 144 112 L 86 112 Z");
const SC = path("M 12 68 L 146 68 L 146 112 L 80 88 L 12 112 Z");

export default function Compatible(ctx) {
  return (
    <Scene style={{ width: 160, height: 120, backgroundColor: "#101010" }}>
      <Path key="pair" d={morph(A, B, ctx.progress, { method: "compatible" })} fill="#ffffff" />
      <Path key="sequence" d={morphSequence([SA, SB, SC], [0, 0.5, 1], ctx.progress, { method: "compatible" })} fill="#22d3ee" />
    </Scene>
  );
}
