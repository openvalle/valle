// error: errors/bounds-decide.tsx:7: TypeError: a value from bounds() is known only after layout, so it cannot decide what the frame holds (if)…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const { width } = bounds("a");
  const wide = [];
  if (width > 30) wide.push(<View key="b" />);
  return <View key="a">{wide}</View>;
}
