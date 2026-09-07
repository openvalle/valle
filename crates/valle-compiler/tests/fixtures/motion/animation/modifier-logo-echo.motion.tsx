export const component = "modifier-logo-echo";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const echoes = defineRepeater({ count: 18, keyPrefix: "echo" });
const ROTATION_ROUTE = path("M 0 0 C 80 -120 160 120 240 0");

export default function ModifierLogoEcho(ctx) {
  const t = ctx.hold.progress;
  const reveal = interpolate(t, [0, 0.16], [0, 1], { easing: "easeOut" });
  const settle = spring({ elapsedFrames: ctx.hold.elapsedFrames, fps: ctx.fps, preset: "bouncy" });
  const drift = wiggle(ctx.localFrame, ctx.fps, {
    seed: 808, frequency: 0.55, amplitude: 11, phase: 0.2,
  });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#050816" }}>
      <View key="violet-glow" className="absolute" style={{ left: 390 + drift, top: 85, width: 500, height: 500, borderRadius: 250, backgroundColor: "#7c3aed", opacity: 0.24, filter: "blur(95px)" }} />
      <View key="cyan-glow" className="absolute" style={{ left: 515 - drift, top: 175, width: 250, height: 250, borderRadius: 125, backgroundColor: "#06b6d4", opacity: 0.22, filter: "blur(62px)" }} />

      {echoes.map((copy) => (
        <View key={copy.key} className="absolute" style={{
          borderStyle: "solid", left: 640 - (78 + copy.index * 24) / 2,
          top: 336 - (78 + copy.index * 24) / 2,
          width: 78 + copy.index * 24,
          height: 78 + copy.index * 24,
          borderRadius: 20 + copy.index * 7,
          borderWidth: copy.index < 3 ? 3 : 1,
          borderColor: copy.index % 2 === 0 ? "#67e8f9" : "#a78bfa",
          backgroundColor: copy.index % 2 === 0 ? "#22d3ee22" : "#8b5cf622",
          opacity: (1 - copy.progress) * trail(t, copy.index, { gap: 0.038, mode: "wrap" }) * 0.72,
          rotate: autoRotate(ROTATION_ROUTE, trail(t, copy.index, { gap: 0.025, mode: "wrap" })),
        }} />
      ))}

      <View key="mark" className="absolute flex items-center justify-center" style={{ borderStyle: "solid", left: 545, top: 241, width: 190, height: 190, borderRadius: 52, backgroundColor: "#090d1fdd", borderWidth: 2, borderColor: "#e0f2fe", opacity: reveal, transform: `scale(${0.72 + settle * 0.28})`, filter: "drop-shadow(0px 22px 42px #000000aa)" }}>
        <Text key="v" style={{ fontSize: 112, color: "#f8fafc" }}>V</Text>
      </View>
      <Text key="title" className="absolute" style={{ left: 0, top: 570, width: 1280, fontSize: 34, letterSpacing: 8, color: "#f8fafc", textAlign: "center", opacity: reveal }}>MODIFIERS / ONE MOTION LANGUAGE</Text>
      <Text key="meta" className="absolute" style={{ left: 0, top: 626, width: 1280, fontSize: 16, letterSpacing: 3, color: "#67e8f9", textAlign: "center", opacity: reveal }}>REPEATER · WIGGLE · TRAIL · AUTO ROTATE</Text>
    </Scene>
  );
}
