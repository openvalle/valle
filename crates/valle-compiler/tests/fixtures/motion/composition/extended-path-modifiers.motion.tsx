export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const STAR = path("M 140 25 L 155 75 L 205 90 L 155 105 L 140 155 L 125 105 L 75 90 L 125 75 Z");
const CROSS = path("M 430 30 L 490 30 L 490 70 L 530 70 L 530 110 L 490 110 L 490 150 L 430 150 L 430 110 L 390 110 L 390 70 L 430 70 Z");
const WAVE = zigzag(line([point(40, 260), point(280, 260)]), { size: 14, ridges: 8 });
const ROUTE = line([point(390, 260), point(570, 260)]);
const RING = strokeToPath(arc(point(320, 190), 34, 0, TAU), 12);
export default function MoreModifiers(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="pucker" d={puckerBloat(STAR, sin(ctx.seconds * 2) * 0.7)} fill="#f472b6" />
      <Path key="twist" d={twist(CROSS, ctx.progress * 1.3)} fill="#8b7bff" />
      <Path key="simplify" d={simplify(WAVE, ctx.progress * 18)} fill="none" stroke="#22d3ee" strokeWidth={3} />
      <Path key="stroke" d={strokeToPath(ROUTE, 8 + ctx.progress * 32)} fill="#ffd000" />
      <Path key="ring" d={RING} fill="#a3e635" />
    </Scene>
  );
}
