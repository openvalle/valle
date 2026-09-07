export function MapPanel(ctx, { regions, x, y, width, height }) {
  const theme = useTheme();
  const geometries = geoPath({ polygons: regions.map((region) => region.rings), projection: "mercator", width: width - 68, height: height - 150, padding: 0.08 });
  return (
    <View key="panel" className="absolute overflow-hidden" style={{ borderStyle: "solid", left: x, top: y, width, height, borderRadius: theme.radius.panel, backgroundColor: theme.colors.panel, borderWidth: 1, borderColor: theme.colors.hairline }}>
      <Text key="kicker" className="absolute" style={{ left: 30, top: 26, fontSize: 15, letterSpacing: 3, color: theme.colors.accent }}>MAP / PREPARED PROJECTION</Text>
      <View key="map" className="absolute" style={{ left: 34, top: 94, width: width - 68, height: height - 150 }}>
        {geometries.map((geometry, index) => <Path key={regions[index].id} d={path(geometry.d)} fill={theme.colors.region} stroke={theme.colors.accent} strokeWidth="3" trimEnd={ctx.hold.progress} style={{ opacity: 0.4 + ctx.enter.progress * 0.6 }} />)}
      </View>
      <Text key="caption" className="absolute" style={{ left: 30, top: height - 44, fontSize: 16, color: theme.colors.ink }}>Synthetic regions from JSON rings</Text>
    </View>
  );
}
