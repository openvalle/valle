export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function Aberration() {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <View key="bar" className="absolute" style={{
      left: 200, top: 100, width: 240, height: 160,
      backgroundColor: "#ffffff", filter: "chromatic-aberration(6px)",
    }} />
  </Scene>;
}
