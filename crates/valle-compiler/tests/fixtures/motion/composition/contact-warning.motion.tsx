export const composition = { width: 64, height: 64, fps: 30, duration: 1 };
const A = path("M 0 0 L 1 0 L 0 -1 Z M 0.75 0.25 L 0.75 1.25 L -0.25 0.25 Z");
const B = path("M 0 0 L 0 1 L 1 0 Z M -0.25 0.25 L -0.25 1.25 L -1.25 0.25 Z");
const anchors = [
  [point(0, 0), point(0, 0)],
  [point(1, 0), point(0, 1)],
  [point(0, -1), point(1, 0)],
  [point(0.75, 0.25), point(-0.25, 0.25)],
  [point(0.75, 1.25), point(-0.25, 1.25)],
  [point(-0.25, 0.25), point(-1.25, 0.25)],
];
export default function Contact(ctx) {
  return <Scene><Path d={morph(A, B, ctx.progress,
    { method: "arcLength", pairs: [[0, 0], [1, 1]], anchors, contactPolicy: "warn" })} fill="#fff" /></Scene>;
}
