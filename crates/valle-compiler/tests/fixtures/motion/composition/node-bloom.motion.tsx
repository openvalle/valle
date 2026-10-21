export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function Bloom() {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#fef3c7" }}>
    <View key="card" className="absolute" style={{
      left: 220, top: 120, width: 200, height: 120, borderRadius: 16,
      backgroundColor: "#1e293b", filter: "bloom(32px 1.5 0.4)",
    }}>
      <View key="hot" className="absolute" style={{
        left: 160, top: 48, width: 24, height: 24, borderRadius: 12, backgroundColor: "#22d3ee",
      }} />
    </View>
  </Scene>;
}
