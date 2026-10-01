export const composition = { width: 160, height: 180, fps: 30, duration: 2 };
const ROWS = Array.from({ length: 4 }, (_, i) => ({
  id: `row${i}`, height: 8 + i % 3 * 4, gap: 2, color: i % 2 ? "#ff5522" : "#3377dd",
  until: i % 2 ? 100 : 5,
}));
export default function Flow(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    {ROWS.map((row) => <View key={row.id} visible={ctx.localFrame < row.until}
      style={{ width: 60, height: row.height, marginBottom: row.gap,
        backgroundColor: row.color, opacity: 0.8 }} />)}
    <View key="reader" className="absolute" style={{
      left: 65, top: 22, width: 5, height: 5,
      backgroundColor: "#ffffff", opacity: bounds("row2").width / 60,
    }} />
  </Scene>;
}
