export const composition = { width: 64, height: 64, fps: 30, duration: 2 };
const FROM = path("M 26 6 L 5 18 L 23 17 L 14 15 L 28 16 Z");
const TO = path("M 7 5 L 19 2 L 28 20 L 28 28 L 15 24 Z");
export default function AllowSelfIntersection(ctx) {
  return (
    <Scene style={{ width: 64, height: 64, backgroundColor: "#101010" }}>
      <Path d={morph(FROM, TO, ctx.progress, { method: "arcLength", allowSelfIntersection: true })} fill="#ffffff" />
    </Scene>
  );
}
