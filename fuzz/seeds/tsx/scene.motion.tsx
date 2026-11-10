export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: seconds < 1 ? 20 : 40 }}>
      <Text>{`time: ${seconds}`}</Text>
    </View>
  );
}
