export const composition = { width: 140, height: 140, fps: 30, duration: 2 };

const SOLID = path("M 20 20 L 120 20 L 120 120 L 20 120 Z M 30 30 L 110 30 L 110 110 L 30 110 Z");
const GROWN = path("M 20 20 L 120 20 L 120 120 L 20 120 Z M 30 30 L 110 30 L 110 110 L 30 110 Z M 22 60 L 28 60 L 28 80 L 22 80 Z");

export default function SiblingHoleBirth(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <Path d={morph(SOLID, GROWN, ctx.progress, {
      pairs: [[0, 0], [1, 1], [null, 2]],
    })} fill="#ffffff" />
  </Scene>;
}
