export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

const LEFT = path("M 40 30 L 120 30 L 120 110 L 40 110 Z M 240 30 L 320 30 L 320 110 L 240 110 Z");
const RIGHT_REVERSED = path("M 280 30 L 360 30 L 360 110 L 280 110 Z M 80 30 L 160 30 L 160 110 L 80 110 Z");
const SOLID = path("M 400 200 L 520 200 L 520 320 L 400 320 Z");
const HOLED = path("M 400 200 L 520 200 L 520 320 L 400 320 Z M 440 240 L 480 240 L 480 280 L 440 280 Z");

export default function PairedAnchors(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="outers" d={morph(LEFT, RIGHT_REVERSED, ctx.progress, {
        pairs: [[0, 1], [1, 0]],
        anchors: [[point(120, 70), point(160, 70)], [point(320, 70), point(360, 70)]],
      })} fill="#ffffff" />
      <Path key="hole" d={morphSequence([SOLID, HOLED, SOLID], [0, 0.5, 1], ctx.progress, {
        pairs: [[0, 0, 0], [null, 1, null]],
        anchors: [[point(520, 260), point(520, 260), point(520, 260)]],
      })} fill="#22d3ee" />
    </Scene>
  );
}
