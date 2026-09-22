export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

const echoes = defineRepeater({ count: 18, keyPrefix: "echo" });
const ROTATION_ROUTE = path("M 0 0 C 120 -180 240 180 360 0");

export default function ModifierLogoEcho(ctx) {
  const t = ctx.progress;
  const reveal = interpolate(t, [0, 0.16], [0, 1], { easing: "easeOut" });
  const settle = spring({ elapsedFrames: ctx.localFrame, fps: ctx.fps, preset: "bouncy" });
  const drift = wiggle(ctx.localFrame, ctx.fps, {
    seed: 808, frequency: 0.55, amplitude: 11, phase: 0.2,
  });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#050816" }}>
      <View key="violet-glow" className="absolute" style={{ left: 585 + drift, top: 127.5, width: 750, height: 750, borderRadius: 375, backgroundColor: "#7c3aed", opacity: 0.24, filter: "blur(142.5px)" }} />
      <View key="cyan-glow" className="absolute" style={{ left: 772.5 - drift, top: 262.5, width: 375, height: 375, borderRadius: 187.5, backgroundColor: "#06b6d4", opacity: 0.22, filter: "blur(93px)" }} />

      {echoes.map((copy) => (
        <View key={copy.key} className="absolute" style={{
          borderStyle: "solid", left: 960 - (117 + copy.index * 36) / 2,
          top: 504 - (117 + copy.index * 36) / 2,
          width: 117 + copy.index * 36,
          height: 117 + copy.index * 36,
          borderRadius: 30 + copy.index * 7,
          borderWidth: copy.index < 3 ? 3 : 1,
          borderColor: copy.index % 2 === 0 ? "#67e8f9" : "#a78bfa",
          backgroundColor: copy.index % 2 === 0 ? "#22d3ee22" : "#8b5cf622",
          opacity: (1 - copy.progress) * trail(t, copy.index, { gap: 0.038, mode: "wrap" }) * 0.72,
          rotate: autoRotate(ROTATION_ROUTE, trail(t, copy.index, { gap: 0.025, mode: "wrap" })),
        }} />
      ))}

      <View key="mark" className="absolute flex items-center justify-center" style={{ borderStyle: "solid", left: 817.5, top: 361.5, width: 285, height: 285, borderRadius: 78, backgroundColor: "#090d1fdd", borderWidth: 3, borderColor: "#e0f2fe", opacity: reveal, transform: `scale(${0.72 + settle * 0.28})`, filter: "drop-shadow(0px 33px 63px #000000aa)" }}>
        <Text key="v" style={{ fontSize: 168, color: "#f8fafc" }}>V</Text>
      </View>
      <Text key="title" className="absolute" style={{ left: 0, top: 855, width: 1920, fontSize: 51, letterSpacing: 12, color: "#f8fafc", textAlign: "center", opacity: reveal }}>MODIFIERS / ONE MOTION LANGUAGE</Text>
      <Text key="meta" className="absolute" style={{ left: 0, top: 939, width: 1920, fontSize: 24, letterSpacing: 4.5, color: "#67e8f9", textAlign: "center", opacity: reveal }}>REPEATER · WIGGLE · TRAIL · AUTO ROTATE</Text>
    </Scene>
  );
}
