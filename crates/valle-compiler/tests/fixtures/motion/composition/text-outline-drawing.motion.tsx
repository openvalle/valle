export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const WORD = textOutline("VALLE", { fontSize: 150, fontWeight: 800, origin: point(60, 240) });
export default function DrawOn(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Path key="word" d={WORD.path} fill="none" stroke="#ffffff" strokeWidth={3} trimEnd={ctx.progress} />
    </Scene>
  );
}
