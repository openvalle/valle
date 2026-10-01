export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Modifiers(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="round" d={roundCorners(path("M 40 40 L 240 40 L 240 200 L 40 200 Z"), 36)} fill="#8b7bff" />
      <Path key="zig" d={zigzag(line([point(300, 80), point(600, 80)]), { size: 10, ridges: 24 })}
        fill="none" stroke="#22d3ee" strokeWidth={3} />
      <Path key="noise" d={noiseDisplace(arc(point(470, 250), 70, 0, TAU), { seed: 3, amount: 16, frequency: 3, phase: ctx.seconds })}
        fill="#f472b6" />
    </Scene>
  );
}
