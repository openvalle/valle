// error: bounds("card/title"): no element has this key path
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const { x } = bounds("card/title");
  return (
    <View key="card">
      <Text key="name" style={{ translate: `${x}px 0px` }}>
        Name
      </Text>
    </View>
  );
}
