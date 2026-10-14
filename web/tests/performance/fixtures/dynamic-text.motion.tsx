export const composition = { width: 640, height: 360, fps: 60, duration: 10 };

// Text whose content changes every frame; packages keep fallback coverage for later frames.
export default function DynamicText(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Text key="time" className="absolute" style={{ left: 40, top: 140, fontSize: 56, color: "#ffffff" }}>
        {`FRAME ${ctx.localFrame}`}
      </Text>
    </Scene>
  );
}
