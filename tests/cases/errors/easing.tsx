// error: errors/easing.tsx:6: TypeError: easing easeOutQuad is not CSS; write cubic-bezier(0.5, 1, 0.89, 1)…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const { seconds } = useCurrentTime();
  const opacity = interpolate(seconds, [0, 1], [0, 1], { easing: "easeOutQuad" });
  return <View style={{ opacity }} />;
}
