export const composition = { width: 160, height: 180, fps: 30, duration: 2 };

const CARDS = Array.from({ length: 4 }, (_, index) => ({
  id: `card${index}`,
  label: `Card ${index}`,
  height: 32 + index * 4,
  color: index % 2 === 0 ? "#3154ad" : "#ac5732",
}));

export default function Cards(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    {CARDS.map((card) => <View key={card.id} style={{ width: 116, height: card.height,
      marginBottom: 3, paddingTop: 3, paddingLeft: 5, backgroundColor: "#202020" }}>
      <View style={{ width: 12, height: 8, backgroundColor: "#e9c456" }} />
      <Group style={{ width: 106, height: 16 }}>
        <Text visible={ctx.localFrame < card.height - 6}
          style={{ fontSize: 12, color: card.color }}>{card.label}</Text>
      </Group>
    </View>)}
  </Scene>;
}
