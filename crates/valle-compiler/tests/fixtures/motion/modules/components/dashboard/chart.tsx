export function ChartPanel(ctx, { rows, x, y, width, height }) {
  const theme = useTheme();
  const values = rows.map((row) => row.value);
  const domain = niceDomain(0, extent(values)[1], 4);
  const xScale = scaleBand({ range: [34, width - 34], count: rows.length, paddingInner: 0.24 });
  const yScale = scaleLinear({ domain, range: [height - 96, 110] });
  const baseline = yScale.map(0);
  return (
    <View key="panel" className="absolute" style={{ borderStyle: "solid", left: x, top: y, width, height, borderRadius: theme.radius.panel, backgroundColor: theme.colors.panel, borderWidth: 1, borderColor: theme.colors.hairline }}>
      <Text key="kicker" className="absolute" style={{ left: 30, top: 26, fontSize: 15, letterSpacing: 3, color: theme.colors.accent }}>CHART / PREPARED SCALE</Text>
      {rows.map((row, index) => {
        const top = yScale.map(row.value);
        const band = xScale.band(index);
        return <View key={row.id} className="absolute" style={{ left: band[0], top, width: xScale.bandwidth(), height: baseline - top, borderRadius: theme.radius.mark, backgroundColor: theme.colors.accent, opacity: 0.28 + ctx.enter.progress * 0.72 }}><Text key={`label-${row.id}`} className="absolute" style={{ left: 0, top: baseline - top + 18, width: xScale.bandwidth(), fontSize: 16, color: theme.colors.ink, textAlign: "center" }}>{row.label}</Text></View>;
      })}
    </View>
  );
}
