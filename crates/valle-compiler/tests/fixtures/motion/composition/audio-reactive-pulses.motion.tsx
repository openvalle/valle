export const composition = { width: 64, height: 64, fps: 30, duration: 2 };
export const controls = { assets: { beat: asset({ kind: "audio", required: true }) } };
const A = audioAnalysis("asset://beat", { bands: 8, rate: 30 });
export default function AudioPulses() { const time = useCurrentTime();
  return <Scene style={{ width: 64, height: 64 }}>
    <View style={{ width: 8, height: A.level(time.seconds) * 60,
      opacity: A.band(3, time.seconds), backgroundColor: "#ffffff" }} />
    <View style={{ width: 8, height: A.onset(time.seconds) * 60,
      opacity: A.beatPhase(time.seconds), backgroundColor: "#ffffff" }} />
  </Scene>;
}
