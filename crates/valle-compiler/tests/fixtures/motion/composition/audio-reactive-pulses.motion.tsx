export const composition = { width: 64, height: 64, fps: 30, duration: 2 };
export const controls = { assets: { beat: asset({ kind: "audio", required: true }) } };
const A = audioAnalysis("asset://beat", { bands: 8, fps: 30 });
export default function AudioPulses(ctx) {
  return <Scene style={{ width: 64, height: 64 }}>
    <View style={{ width: 8, height: A.level(ctx.seconds) * 60,
      opacity: A.band(3, ctx.seconds), backgroundColor: "#ffffff" }} />
    <View style={{ width: 8, height: A.onset(ctx.seconds) * 60,
      opacity: A.beatPhase(ctx.seconds), backgroundColor: "#ffffff" }} />
  </Scene>;
}
