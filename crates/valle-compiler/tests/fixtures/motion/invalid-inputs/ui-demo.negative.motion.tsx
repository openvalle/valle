export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export default function InteractiveUi() {
  return <View key="button" onClick={() => publish()} style={{ width: 240, height: 80, backgroundColor: "#2563eb" }} />;
}
