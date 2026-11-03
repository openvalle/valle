// error: errors/bounds-layout.tsx:8:7: style width: a value from bounds() is known only after layout…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View>
      <View key="a" style={{ width: 20, height: 20 }} />
      <View style={{ width: bounds("a").width, height: 20 }} />
    </View>
  );
}
