export function MapPanel(ctx, { regions, x, y, width, height }) {
  const theme = useTheme();
  const { panel, accent, ink, hairline, region: regionFill } = theme.colors;
  const geometries = geoPath({
    polygons: regions.map((region) => region.rings),
    projection: "mercator",
    width: width - 68,
    height: height - 150,
    padding: 0.08,
  });
  return (
    <View key="panel" className="absolute" style={{ borderStyle: "solid", left: x, top: y, width, height, borderRadius: theme.radius.panel, backgroundColor: panel, borderWidth: 1, borderColor: hairline, overflow: "hidden" }}>
      <Text key="kicker" className="absolute" style={{ left: 30, top: 26, fontSize: 15, letterSpacing: 3, color: accent }}>MAP / PREPARED PROJECTION</Text>
      <View key="map" className="absolute" style={{ left: 34, top: 94, width: width - 68, height: height - 150 }}>
        {geometries.map((geometry, index) => {
          const regionData = regions[index];
          return <Path key={regionData.id} d={path(geometry.d)} fill={regionFill} stroke={accent} strokeWidth="3" style={{ opacity: 0.4 + ctx.enter.progress * 0.6 }} />;
        })}
      </View>
      <Text key="caption" className="absolute" style={{ left: 30, top: height - 44, fontSize: 16, color: ink }}>Synthetic regions from JSON rings</Text>
    </View>
  );
}
