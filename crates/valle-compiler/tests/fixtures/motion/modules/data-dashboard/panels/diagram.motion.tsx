export function DiagramPanel(ctx, { nodes, edges, x, y, width, height }) {
  const theme = useTheme();
  const { panel, panelRaised, accent, ink, hairline } = theme.colors;
  const sizes = nodes.map(() => [156, 70]);
  const pairs = edges.map((edge) => [edge.from, edge.to]);
  const layout = graphLayout({ sizes, edges: pairs, nodeGap: 34, rankGap: 58 });
  return (
    <View key="panel" className="absolute" style={{ borderStyle: "solid", left: x, top: y, width, height, borderRadius: theme.radius.panel, backgroundColor: panel, borderWidth: 1, borderColor: hairline }}>
      <Text key="kicker" className="absolute" style={{ left: 30, top: 26, fontSize: 15, letterSpacing: 3, color: accent }}>DIAGRAM / PREPARED GRAPH</Text>
      {edges.map((edge) => {
        const from = layout.centers[edge.from];
        const to = layout.centers[edge.to];
        const route = line([point(58 + from[0], 116 + from[1]), point(58 + to[0], 116 + to[1])]);
        return <Path key={edge.id} d={route} fill="none" stroke={accent} strokeWidth="3" trimEnd={ctx.hold.progress} style={{ opacity: 0.7 }} />;
      })}
      {nodes.map((node, index) => {
        const center = layout.centers[index];
        return (
          <View key={node.id} className="absolute" style={{ borderStyle: "solid", left: 58 + center[0] - 78, top: 116 + center[1] - 35, width: 156, height: 70, borderRadius: theme.radius.node, backgroundColor: panelRaised, borderWidth: 2, borderColor: accent, opacity: 0.35 + ctx.enter.progress * 0.65 }}>
            <Text key={`label-${node.id}`} className="absolute" style={{ left: 8, top: 23, width: 140, fontSize: 17, color: ink, textAlign: "center" }}>{node.label}</Text>
          </View>
        );
      })}
    </View>
  );
}
