export const composition = { width: 160, height: 180, fps: 30, duration: 2 };

const CARDS = Array.from({ length: 4 }, (_, index) => ({
  id: `card${index}`,
  label: `Card ${index}`,
  height: 32 + index * 4,
  color: index % 2 === 0 ? "#3154ad" : "#ac5732",
}));

function Card(ctx, { item }) {
  const height = item.height;
  const color = item.color;
  const label = item.label;
  return <View key="body" className={height > 36 ? "flex flex-col opacity-50" : "flex flex-col"}
    style={{ width: 116, height, marginBottom: 3, paddingTop: 3, paddingLeft: 5,
      backgroundColor: "#202020" }}>
    <View style={{ width: 12, height: 8, backgroundColor: "#e9c456" }} />
    <Group style={{ width: 106, height: 16 }}>
      <Text visible={ctx.localFrame < height - 6}
        style={{ fontSize: 12, color }}>{label}</Text>
    </Group>
  </View>;
}

export default function Cards(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    {CARDS.map((card) => <Card key={card.id} item={card} />)}
  </Scene>;
}
