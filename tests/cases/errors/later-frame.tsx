// error: errors/later-frame.tsx:6:10: style width: wide is not a length
export const composition = { width: 64, height: 64, fps: 1, duration: 3 };

export default function Scene() {
  const { seconds } = useCurrentTime();
  return <View style={{ width: seconds < 2 ? 10 : "wide" }} />;
}
