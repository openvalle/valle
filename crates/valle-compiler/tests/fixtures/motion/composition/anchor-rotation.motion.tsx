export const composition = { width: 640, height: 240, fps: 30, duration: 2 };
const TOP_A = arc(point(140, 60), 45, 0, TAU);
const TOP_B = arc(point(500, 60), 45, 0, TAU);
const BOTTOM_A = arc(point(140, 170), 45, 0, TAU);
const BOTTOM_B = arc(point(500, 170), 45, 0, TAU);
export default function AnchorRotation(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="anchored" d={morph(TOP_A, TOP_B, ctx.progress, {
        anchors: [[point(185, 60), point(500, 105)]],
      })} fill="#ffffff" />
      <Path key="automatic" d={morph(BOTTOM_A, BOTTOM_B, ctx.progress)} fill="#22d3ee" />
    </Scene>
  );
}
