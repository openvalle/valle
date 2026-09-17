export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export default function RuntimeTicks(ctx) {
  const scale = scaleLinear({ domain: [0, 100 + ctx.hold.progress], range: [620, 120] });
  return <View key="bar" style={{ position: "absolute", left: 40, top: scale.map(50), width: 80, height: 200 }} />;
}
