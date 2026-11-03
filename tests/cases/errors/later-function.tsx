// error: errors/later-function.tsx:5: TypeError: particles() is not supported yet…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const dots = particles(0, { seed: 1, count: 4 });
  return <View style={{ width: dots.length }} />;
}
