export const composition = { width: 160, height: 180, fps: 30, duration: 2 };
const ROWS = Array.from({ length: 4 }, (_, i) => ({
  id: `row${i}`, label: `Line ${i}`, color: i % 2 ? "#ac5732" : "#3154ad",
}));
function Label(ctx, { item }) {
  return <Text key="body" style={{ fontSize: 14, color: item.color }}>{item.label}</Text>;
}
export default function Labels(ctx) {
  return <Scene className="flex flex-col h-full w-full" style={{ backgroundColor: "#101010" }}>
    {ROWS.map((row) => <Label key={row.id} item={row} />)}
  </Scene>;
}
