export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function RichUnits(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Text key="t" split="char" className="absolute" style={{ left: 60, top: 140, fontSize: 72, color: "#ffffff" }}
        perUnit={{ translate: point(0, sin(ctx.unit.index * 0.8 + ctx.seconds * 6) * 10) }}>
        {"Hello "}<Span style={{ color: "#ff4a24", fontWeight: 700 }}>Valle</Span>
      </Text>
    </Scene>
  );
}
