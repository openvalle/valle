export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const rays = Array.from({ length: 72 }, (_, i) => i);

export default function RadialBars() {
  const { seconds: t } = useCurrentTime();
  const level = Math.min(0.9, (Math.sin(t * 4) + 1) * 0.4);
  return <Scene className="relative h-full w-full">
    {rays.map(i => <View key={`ray-${i}`} style={{
      position: "absolute",
      left: 320 + Math.cos(i * Math.PI / 36 + t * 0.025) * (70 + level * 8),
      top: 180 + Math.sin(i * Math.PI / 36 + t * 0.025) * (70 + level * 8) - 1.5,
      width: 12 + (Math.sin(i * 0.73 + t * 3) + 1) * 28,
      height: 3,
      borderRadius: 1.5,
      backgroundColor: i % 6 === 0 ? "#DEB66F" : "#6BDACB",
      rotate: `${(i * Math.PI / 36 + t * 0.025) * 180 / Math.PI}deg`,
      transformOrigin: "0% 50%",
      opacity: 0.2 + level * 0.7,
    }} />)}
  </Scene>;
}
