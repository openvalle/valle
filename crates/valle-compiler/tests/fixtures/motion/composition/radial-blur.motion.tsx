export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function RadialBlur() {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <View key="bar" className="absolute" style={{
      left: 280, top: 160, width: 80, height: 40,
      backgroundColor: "#ffffff", filter: "radial-blur(40px 20px 20px)",
    }} />
  </Scene>;
}
