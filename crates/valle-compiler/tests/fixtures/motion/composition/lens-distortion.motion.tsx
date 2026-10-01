export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function LensDistortion() {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <View key="frame" className="absolute" style={{
      left: 200, top: 100, width: 240, height: 160,
      backgroundColor: "#303030", filter: "lens-distortion(-0.3 0)",
    }}>
      <View key="left" className="absolute" style={{
        left: 52, top: 20, width: 12, height: 120, backgroundColor: "#ffffff",
      }} />
      <View key="right" className="absolute" style={{
        left: 176, top: 20, width: 12, height: 120, backgroundColor: "#ffffff",
      }} />
    </View>
  </Scene>;
}
