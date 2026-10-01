export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function Bloom() {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}
    bloom={{ threshold: 0.8, intensity: 1.2, radius: 48 }}>
    <View key="hot" className="absolute" style={{ left: 120, top: 150, width: 60, height: 60, backgroundColor: "#ffffff" }} />
    <View key="dim" className="absolute" style={{ left: 460, top: 150, width: 60, height: 60, backgroundColor: "#404040" }} />
  </Scene>;
}
