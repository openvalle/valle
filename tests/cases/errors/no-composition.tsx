// error: errors/no-composition.tsx: a scene exports its composition, such as `export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 }`
export default function Scene() {
  return <View />;
}
