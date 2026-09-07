export const component = "map-atlas";

export const controls = defineControls({
  assets: { brandFont: asset({ kind: "font", required: true }) },
  props: {
    ocean: color({ default: "#07111f" }),
    land: color({ default: "#15324a" }),
    accent: color({ default: "#22d3ee" }),
    signal: color({ default: "#f59e0b" }),
    label: color({ default: "#dbeafe" }),
  },
});

// Synthetic geography: it deliberately exercises a hole and an antimeridian crossing without
// pretending to be a political boundary dataset.
const REGIONS = [
  { id: "aurora", name: "AURORA", rings: [[[-156, 54], [-118, 68], [-82, 55], [-96, 31], [-138, 28]], [[-130, 50], [-113, 55], [-106, 43], [-126, 39]]], priority: 6 },
  { id: "meridian", name: "MERIDIAN", rings: [[[-48, 63], [-4, 67], [24, 47], [4, 26], [-37, 34]]], priority: 5 },
  { id: "solace", name: "SOLACE", rings: [[[34, 54], [82, 70], [116, 49], [94, 24], [47, 28]]], priority: 4 },
  { id: "delta", name: "DELTA", rings: [[[-22, 13], [20, 22], [41, -12], [12, -39], [-27, -22]]], priority: 3 },
  { id: "pelagic", name: "PELAGIC", rings: [[[76, 8], [132, 17], [147, -21], [108, -43], [70, -23]]], priority: 2 },
  { id: "dateline", name: "DATELINE", rings: [[[170, 18], [-170, 16], [-174, -12], [173, -16]]], priority: 1 },
];

const MAIN = geoPath({ polygons: REGIONS.map((region) => region.rings), projection: "mercator", width: 850, height: 510, padding: 0.055, clip: [0, 0, 850, 510] });
const MINI = geoPath({ polygons: REGIONS.map((region) => region.rings), projection: "mercator", width: 250, height: 136, padding: 0.08 });
const LABEL_METRICS = REGIONS.map((region) => measureText(region.name, { style: "font-family: 'asset://brandFont'; font-size: 15px; letter-spacing: 1.8px" }));
const LABELS = placeLabels(REGIONS.map((region, index) => ({ point: MAIN[index].centroid, size: [LABEL_METRICS[index].width + 18, 25], priority: region.priority })), { bounds: [8, 8, 842, 502], padding: 5 });

const FLOW_PAIRS = [[0, 2], [1, 4], [3, 5]];
const DENSITY = Array.from({ length: 1200 }, (_, index) => point(12 + (index % 60) * 13.7, 12 + Math.floor(index / 60) * 24.8));
const FLOWS = FLOW_PAIRS.map((pair, index) => {
  const from = MAIN[pair[0]].centroid;
  const to = MAIN[pair[1]].centroid;
  const middle = (from[0] + to[0]) / 2;
  const lift = 68 + index * 18;
  return cubic(point(from[0], from[1]), point(middle, from[1] - lift), point(middle, to[1] - lift), point(to[0], to[1]));
});

function Region(ctx, { geometry, fill, stroke, strokeWidth, index }) {
  const reveal = interpolate(ctx.enter.progress - index * 0.035, [0, 0.48], [0, 1], { easing: "easeOut" });
  return <Path key="shape" d={path(geometry.d)} fill={fill} stroke={stroke} strokeWidth={strokeWidth} style={{ opacity: 0.34 + reveal * 0.66 }} />;
}

function Marker(ctx, { pointValue, color, index }) {
  const reveal = interpolate(ctx.hold.progress, [0.08 + index * 0.08, 0.22 + index * 0.08], [0, 1], { easing: "easeOut" });
  return (
    <View key="marker" className="absolute" style={{ left: pointValue[0] - 8, top: pointValue[1] - 8, width: 16, height: 16, borderRadius: 8, backgroundColor: color, opacity: reveal, transform: `scale(${0.45 + reveal * 0.55})`, filter: `drop-shadow(0px 0px ${8 + reveal * 12}px ${color})` }}>
      <View key="core" className="absolute" style={{ left: 5, top: 5, width: 6, height: 6, borderRadius: 3, backgroundColor: "#ffffff" }} />
    </View>
  );
}

function Flow(ctx, { route, color, index }) {
  const reveal = interpolate(ctx.hold.progress, [0.1 + index * 0.12, 0.52 + index * 0.12], [0, 1], { easing: "easeInOut" });
  return <Path key="route" d={route} fill="none" stroke={color} strokeWidth="3" strokeLinecap="round" trimEnd={reveal} arrowEnd="triangle" arrowSize="11" style={{ opacity: 0.92 }} />;
}

function MapLabel(ctx, { region, placement, color, index }) {
  const reveal = interpolate(ctx.hold.progress, [0.22 + index * 0.035, 0.4 + index * 0.035], [0, 1]);
  return (
    <View key="label" className="absolute" style={{ borderStyle: "solid", left: placement.x, top: placement.y, height: 25, paddingLeft: 9, paddingRight: 9, borderRadius: 12, backgroundColor: "#020617c9", borderWidth: 1, borderColor: "#ffffff1f", opacity: placement.visible ? reveal : 0 }}>
      <Text key="text" style={{ marginTop: 4, fontFamily: "asset://brandFont", fontSize: 15, letterSpacing: 1.8, color }}>{region.name}</Text>
    </View>
  );
}

function WorldMap(ctx, { geometry, width, height, radius, gridOne, gridTwo, strokeWidth, ocean, land, accent }) {
  return (
    <View key="map" className="absolute" style={{ borderStyle: "solid", left: 0, top: 0, width, height, borderRadius: radius, backgroundColor: ocean, overflow: "hidden", borderWidth: 1, borderColor: accent }}>
      <View key="graticule-a" className="absolute" style={{ left: 0, top: gridOne, width, height: 1, backgroundColor: accent, opacity: 0.14 }} />
      <View key="graticule-b" className="absolute" style={{ left: 0, top: gridTwo, width, height: 1, backgroundColor: accent, opacity: 0.14 }} />
      {REGIONS.map((region, index) => <Region key={`region-${region.id}`} geometry={geometry[index]} fill={land} stroke={accent} strokeWidth={strokeWidth} index={index} />)}
    </View>
  );
}

export default function MapAtlas(ctx, props) {
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#030712", fontFamily: "asset://brandFont" }} camera={{ center: point(640, 360), zoom: 1, rotation: 0 }}>
      <World key="world">
        <View key="main-host" className="absolute" style={{ left: 70, top: 148, width: 850, height: 510, filter: "drop-shadow(0px 26px 52px #00000099)" }}>
          <WorldMap key="main-map" geometry={MAIN} width={850} height={510} radius={28} gridOne={170} gridTwo={340} strokeWidth="2" ocean={props.ocean} land={props.land} accent={props.accent} />
          <GeometryBatch key="density-samples" geometry="circle" positions={DENSITY} sizes={1.4} fills="#67e8f9" opacities={0.08} style={{ position: "absolute", left: 0, top: 0, width: 850, height: 510 }} />
          {FLOWS.map((route, index) => <Flow key={`flow-${index}`} route={route} color={props.signal} index={index} />)}
          {REGIONS.map((region, index) => <MapLabel key={`label-${region.id}`} region={region} placement={LABELS[index]} color={props.label} index={index} />)}
          {REGIONS.slice(0, 5).map((region, index) => <Marker key={`marker-${region.id}`} pointValue={MAIN[index].centroid} color={props.accent} index={index} />)}
        </View>
        <View key="mini-host" className="absolute" style={{ left: 950, top: 474, width: 250, height: 136, filter: "drop-shadow(0px 18px 32px #00000088)" }}>
          <WorldMap key="mini-map" geometry={MINI} width={250} height={136} radius={14} gridOne={45} gridTwo={91} strokeWidth="1" ocean="#160b24" land="#3b1c54" accent="#c084fc" />
        </View>
      </World>
      <Screen key="hud">
        <Text key="kicker" className="absolute" style={{ left: 70, top: 48, fontFamily: "asset://brandFont", fontSize: 16, letterSpacing: 4, color: props.accent }}>SYNTHETIC MOBILITY ATLAS / 01</Text>
        <Text key="title" className="absolute" style={{ left: 68, top: 76, width: 820, fontFamily: "asset://brandFont", fontSize: 46, color: props.label }}>One projection. Replaceable parts.</Text>
        <View key="legend" className="absolute flex items-center" style={{ borderStyle: "solid", left: 934, top: 70, width: 280, height: 46, borderRadius: 18, backgroundColor: "#020617dd", borderWidth: 1, borderColor: "#ffffff1c" }}>
          <View key="legend-line" style={{ marginLeft: 18, width: 46, height: 3, backgroundColor: props.signal }} />
          <Text key="legend-copy" style={{ marginLeft: 12, fontFamily: "asset://brandFont", fontSize: 15, color: props.label }}>PREPARED FLOW</Text>
        </View>
      </Screen>
    </Scene>
  );
}
