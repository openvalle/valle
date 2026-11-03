// error: errors/spring.tsx:6: TypeError: spring takes a preset or mass, not both…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const { seconds } = useCurrentTime();
  const x = spring(seconds, { preset: "gentle", mass: 2 });
  return <View style={{ opacity: x }} />;
}
