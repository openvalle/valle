export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
function Bar(ctx) {
  return <View key="bar" className="absolute" style={{ left: 20, top: 160, width: 100 + ctx.seconds * 100,
    height: 40, backgroundColor: "#ffffff" }} />;
}
export default function Scope(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <TimeScope key="scope" offset={0.5} speed={2}>
        <Bar key="bar" />
      </TimeScope>
    </Scene>
  );
}
