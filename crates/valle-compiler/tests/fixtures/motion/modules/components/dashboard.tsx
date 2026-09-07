import { ChartPanel } from "./dashboard/chart";
import { DiagramPanel } from "./dashboard/diagram";
import { MapPanel } from "./dashboard/map";
import { brandTheme } from "./dashboard/theme";

export const component = "release-dashboard";

export const controls = defineControls({
  props: {
    accentStrength: number({ default: 1, min: 0.25, max: 1 }),
  },
  data: {
    metrics: array(
      record({ id: string(), label: string(), value: number({ min: 0 }) }),
      { minItems: 1, maxItems: 8, key: "id" },
    ),
    regions: array(
      record({
        id: string(),
        rings: array(array(point(), { minItems: 3, maxItems: 16 }), { minItems: 1, maxItems: 4 }),
      }),
      { minItems: 1, maxItems: 6, key: "id" },
    ),
    nodes: array(record({ id: string(), label: string() }), { minItems: 1, maxItems: 8, key: "id" }),
    edges: array(
      record({ id: string(), from: number({ min: 0 }), to: number({ min: 0 }) }),
      { maxItems: 12, key: "id" },
    ),
  },
});

export default function ReleaseDashboard(ctx, props, signals, data) {
  const reveal = interpolate(ctx.enter.progress, [0, 1], [0, 1], { easing: "easeOut" });
  return (
    <ThemeProvider value={brandTheme}>
      <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: useTheme().colors.background }}>
        <View key="aurora" className="absolute" style={{ left: 1180, top: -300, width: 980, height: 980, borderRadius: 500, backgroundColor: useTheme().colors.accent, opacity: 0.08 * props.accentStrength, filter: "blur(150px)" }} />
        <Text key="eyebrow" className="absolute" style={{ left: 64, top: 42, fontSize: 18, letterSpacing: 4, color: useTheme().colors.accent, opacity: reveal }}>DATA SYSTEM</Text>
        <Text key="title" className="absolute" style={{ left: 62, top: 72, width: 1300, fontSize: 48, color: useTheme().colors.ink, opacity: reveal, translate: point(0, (1 - reveal) * 24) }}>One dataset. Three visual languages.</Text>
        <ChartPanel key="chart" rows={data.metrics} x={54} y={166} width={570} height={760} />
        <MapPanel key="map" regions={data.regions} x={675} y={166} width={570} height={760} />
        <DiagramPanel key="diagram" nodes={data.nodes} edges={data.edges} x={1296} y={166} width={570} height={760} />
        <Text key="footnote" className="absolute" style={{ left: 64, top: 1018, width: 1700, fontSize: 18, color: useTheme().colors.muted }}>Chart, geo and graph compile to ordinary Motion nodes · prepared JSON · random-seek pure</Text>
      </Scene>
    </ThemeProvider>
  );
}
