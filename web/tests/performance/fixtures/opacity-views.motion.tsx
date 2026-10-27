export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const upper = defineRepeater({ count: 2048, keyPrefix: "upper" });
const lower = defineRepeater({ count: 2048, keyPrefix: "lower" });

export default function OpacityViews() {
  const time = useCurrentTime();
  return (
    <Scene style={{ width: 640, height: 360, backgroundColor: "#18202a" }}>
      <Group style={{ width: 640, height: 360, translate: point(time.seconds * 2, 0) }}>
        {upper.map((row) => (
          <View key={row.key} style={{ position: "absolute", left: 18 + (row.index % 64) * 9,
            top: 16 + Math.floor(row.index / 64) * 5, width: 4, height: 3,
            backgroundColor: "#4ba8e5", opacity: 0.63 }} />
        ))}
        {lower.map((row) => (
          <View key={row.key} style={{ position: "absolute", left: 18 + (row.index % 64) * 9,
            top: 176 + Math.floor(row.index / 64) * 5, width: 4, height: 3,
            backgroundColor: "#4ba8e5", opacity: 0.63 }} />
        ))}
      </Group>
    </Scene>
  );
}
