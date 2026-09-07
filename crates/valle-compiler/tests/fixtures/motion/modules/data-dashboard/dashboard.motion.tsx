import { ChartPanel } from "./panels/chart.motion";
import { DiagramPanel } from "./panels/diagram.motion";
import { MapPanel } from "./panels/map.motion";
import { brandTheme } from "./theme.motion";

export const component = "DataDashboard";

export const controls = defineControls({
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
    nodes: array(
      record({ id: string(), label: string() }),
      { minItems: 1, maxItems: 8, key: "id" },
    ),
    edges: array(
      record({ id: string(), from: number({ min: 0 }), to: number({ min: 0 }) }),
      { maxItems: 12, key: "id" },
    ),
  },
});

export default function DataDashboard(ctx, props, signals, data) {
  return (
    <ThemeProvider value={brandTheme}>
      <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: useTheme().colors.background }}>
        <Text key="eyebrow" className="absolute" style={{ left: 64, top: 44, fontSize: 18, letterSpacing: 4, color: useTheme().colors.accent }}>
          DATA + COMPILE-TIME THEME
        </Text>
        <Text key="title" className="absolute" style={{ left: 62, top: 74, width: 1300, fontSize: 48, color: useTheme().colors.ink }}>
          Data becomes ordinary Motion nodes
        </Text>
        <ChartPanel key="chart" rows={data.metrics} x={54} y={166} width={570} height={760} />
        <MapPanel key="map" regions={data.regions} x={675} y={166} width={570} height={760} />
        <DiagramPanel key="diagram" nodes={data.nodes} edges={data.edges} x={1296} y={166} width={570} height={760} />
        <Text key="footnote" className="absolute" style={{ left: 64, top: 1018, width: 1700, fontSize: 18, color: useTheme().colors.muted }}>
          Chart, geographic projection, and graph layout compile to View, Text, and Path — no domain IR.
        </Text>
      </Scene>
    </ThemeProvider>
  );
}
