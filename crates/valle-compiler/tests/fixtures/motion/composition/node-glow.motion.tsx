export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function Glow() {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <View key="dot" className="absolute" style={{
      left: 280, top: 140, width: 80, height: 80, borderRadius: 40,
      backgroundColor: "#22d3ee", filter: "glow(24px 1.5 #22d3ee)",
    }} />
  </Scene>;
}
