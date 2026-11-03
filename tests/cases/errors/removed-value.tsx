// error: errors/removed-value.tsx:6: TypeError: TAU is removed: write 2 * Math.PI…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const { seconds } = useCurrentTime();
  return <View style={{ rotate: `${seconds * TAU}rad` }} />;
}
