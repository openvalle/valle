export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function PaintParameters() {
  const t = useCurrentTime();
  return <Scene style={{ width: 640, height: 360, backgroundColor: "#111827" }}>
    <View key="color" style={{ position: "absolute", left: 32, top: 40, width: 240, height: 120,
      backgroundColor: interpolate(t.seconds, [0, 1, 2], ["#b94332", "#4f83dd", "#2d9b7d"]) }} />
    <View key="fixed" style={{ position: "absolute", left: 336, top: 40, width: 240, height: 120, backgroundColor: "#4f83dd" }} />
    <View key="alpha" style={{ position: "absolute", left: 32, top: 220, width: 544, height: 96,
      backgroundColor: interpolate(t.seconds, [0, 1, 2], ["#f2bb4420", "#f2bb44ff", "#f2bb4400"]) }} />
  </Scene>;
}
