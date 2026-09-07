import { releaseTheme } from "./shared-theme";

export const component = "route-network";

export const controls = defineControls({
  props: { accentStrength: number({ default: 1, min: 0.25, max: 1 }) },
  cues: { narration: spanCue() },
});

const REGIONS = [
  { id: "west", rings: [[[-150, 56], [-103, 67], [-76, 42], [-111, 24], [-150, 56]]] },
  { id: "mid", rings: [[[-54, 58], [-8, 67], [22, 36], [-32, 24], [-54, 58]]] },
  { id: "east", rings: [[[43, 56], [93, 67], [126, 34], [71, 22], [43, 56]]] },
  { id: "south", rings: [[[-18, 12], [39, 20], [54, -28], [2, -44], [-30, -13], [-18, 12]]] },
];
const MAP = geoPath({ polygons: REGIONS.map((region) => region.rings), projection: "mercator", width: 760, height: 520, padding: 0.08 });
const ROUTES = [
  cubic(point(248, 430), point(480, 172), point(710, 168), point(910, 362)),
  cubic(point(420, 560), point(690, 258), point(1020, 248), point(1370, 492)),
  cubic(point(570, 300), point(820, 104), point(1160, 126), point(1540, 332)),
];
const GRID = Array.from({ length: 1600 }, (_, index) => point(80 + (index % 80) * 22, 154 + Math.floor(index / 80) * 36));
const NETWORK = Array.from({ length: 1600 }, (_, index) => {
  const column = index % 80;
  const row = Math.floor(index / 80);
  return point(180 + row * 78 + (column % 8) * 7, 185 + column * 8.4 + (row % 4) * 18);
});
const NODES = [
  { id: "ingest", label: "INGEST", x: 1090, y: 250 },
  { id: "reason", label: "REASON", x: 1320, y: 430 },
  { id: "render", label: "RENDER", x: 1580, y: 270 },
];

export default function RouteNetwork(ctx, props, signals) {
  const reveal = interpolate(ctx.enter.progress, [0, 1], [0, 1], { easing: "easeOut" });
  const flow = ctx.hold.progress;
  const center = interpolate(flow, [0, 1], [760, 1160], { easing: "easeInOut" });
  return (
    <ThemeProvider value={releaseTheme}>
      <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: useTheme().colors.background }} camera={{ center: point(center, 540), zoom: 1, rotation: 0 }}>
        <World key="world">
          <GeometryBatch
            key="signal-field"
            geometry="circle"
            positions={field({ from: GRID, to: NETWORK, progress: flow, stagger: 0.0002 })}
            sizes={field({ from: 1.2, to: 3.2, progress: flow })}
            fills={field({ from: "#164e63", to: "#f472b6", progress: flow })}
            opacities={field({ from: 0.08, to: 0.42, progress: flow })}
          />
          <View key="map-card" className="absolute overflow-hidden" style={{ borderStyle: "solid", left: 100, top: 190, width: 820, height: 620, borderRadius: useTheme().radius.panel, backgroundColor: useTheme().colors.panel, borderWidth: 1, borderColor: useTheme().colors.hairline }}>
            <Text key="map-kicker" className="absolute" style={{ left: 34, top: 28, fontSize: 15, letterSpacing: 3, color: useTheme().colors.accent }}>01 / GEOGRAPHY</Text>
            {MAP.map((geometry, index) => <Path key={REGIONS[index].id} d={path(geometry.d)} fill={index === 2 ? "#312e81" : "#12324b"} stroke={useTheme().colors.accent} strokeWidth="3" trimEnd={reveal} style={{ position: "absolute", left: 30, top: 72, opacity: 0.35 + reveal * 0.65 }} />)}
          </View>
          {ROUTES.map((route, index) => <Path key={`route-${index}`} d={route} fill="none" stroke={index === 1 ? useTheme().colors.signal : useTheme().colors.accent} strokeWidth="5" strokeDasharray={[14, 12]} strokeDashoffset={-flow * (54 + index * 17)} strokeLinecap="round" trimEnd={interpolate(flow - index * 0.08, [0, 0.6], [0, 1])} arrowEnd="triangle" arrowSize="14" />)}
          <View key="network-card" className="absolute" style={{ borderStyle: "solid", left: 1010, top: 180, width: 720, height: 640, borderRadius: useTheme().radius.panel, backgroundColor: useTheme().colors.panel, borderWidth: 1, borderColor: useTheme().colors.hairline }}>
            <Text key="network-kicker" className="absolute" style={{ left: 34, top: 28, fontSize: 15, letterSpacing: 3, color: useTheme().colors.signal }}>02 / NETWORK</Text>
          </View>
          {NODES.map((node, index) => <View key={node.id} className="absolute" style={{ borderStyle: "solid", left: node.x, top: node.y, width: 180, height: 82, borderRadius: useTheme().radius.node, backgroundColor: useTheme().colors.panelRaised, borderWidth: 2, borderColor: index === 1 ? useTheme().colors.signal : useTheme().colors.accent, opacity: interpolate(flow - index * 0.1, [0, 0.28], [0, 1]), transform: `scale(${0.8 + interpolate(flow - index * 0.1, [0, 0.28], [0, 1]) * 0.2})` }}><Text key={`label-${node.id}`} className="absolute" style={{ left: 0, top: 28, width: 180, fontSize: 18, letterSpacing: 2, color: useTheme().colors.ink, textAlign: "center" }}>{node.label}</Text></View>)}
        </World>
        <Screen key="hud">
          <Text key="eyebrow" className="absolute" style={{ left: 64, top: 44, fontSize: 17, letterSpacing: 4, color: useTheme().colors.accent }}>ROUTE / NETWORK NARRATIVE</Text>
          <Text key="title" className="absolute" style={{ left: 62, top: 78, width: 1200, fontSize: 52, color: useTheme().colors.ink, opacity: props.accentStrength }}>The signal moves where the story needs it.</Text>
          <Text key="cue" className="absolute" style={{ right: 66, top: 54, fontSize: 14, letterSpacing: 2, color: useTheme().colors.signal, opacity: 0.55 + signals.narration.progress * 0.45 }}>NARRATION ALIGNED</Text>
        </Screen>
      </Scene>
    </ThemeProvider>
  );
}
