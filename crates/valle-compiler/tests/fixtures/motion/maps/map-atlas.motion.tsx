export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
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

const MAIN = geoPath({ polygons: REGIONS.map((region) => region.rings), projection: "mercator", width: 1275, height: 765, padding: 0.055, clip: [0, 0, 1275, 765] });
const MINI = geoPath({ polygons: REGIONS.map((region) => region.rings), projection: "mercator", width: 375, height: 204, padding: 0.08 });
const LABEL_METRICS = REGIONS.map((region) => measureText(region.name, { fontFamily: "asset://brandFont", fontSize: 22.5, letterSpacing: 2.7 }));
const LABELS = placeLabels(REGIONS.map((region, index) => ({ point: MAIN[index].centroid, size: [LABEL_METRICS[index].width + 27, 37.5], priority: region.priority })), { bounds: [12, 12, 1263, 753], padding: 7.5 });

const FLOW_PAIRS = [[0, 2], [1, 4], [3, 5]];
const DENSITY = Array.from({ length: 1200 }, (_, index) => point(18 + (index % 60) * 20.55, 18 + Math.floor(index / 60) * 37.2));
const FLOWS = FLOW_PAIRS.map((pair, index) => {
  const from = MAIN[pair[0]].centroid;
  const to = MAIN[pair[1]].centroid;
  const middle = (from[0] + to[0]) / 2;
  const lift = 102 + index * 27;
  return cubic(point(from[0], from[1]), point(middle, from[1] - lift), point(middle, to[1] - lift), point(to[0], to[1]));
});

function Region(ctx, { geometry, fill, stroke, strokeWidth, index }) {
  const reveal = interpolate(ctx.enter.progress - index * 0.035, [0, 0.48], [0, 1], { easing: "easeOut" });
  return <Path key="shape" d={path(geometry.d)} fill={fill} stroke={stroke} strokeWidth={strokeWidth} style={{ opacity: 0.34 + reveal * 0.66 }} />;
}

function Marker(ctx, { pointValue, color, index }) {
  const reveal = interpolate(ctx.hold.progress, [0.08 + index * 0.08, 0.22 + index * 0.08], [0, 1], { easing: "easeOut" });
  return (
    <View key="marker" className="absolute" style={{ left: pointValue[0] - 12, top: pointValue[1] - 12, width: 24, height: 24, borderRadius: 12, backgroundColor: color, opacity: reveal, transform: `scale(${0.45 + reveal * 0.55})`, filter: `drop-shadow(0px 0px ${12 + reveal * 18}px ${color})` }}>
      <View key="core" className="absolute" style={{ left: 7.5, top: 7.5, width: 9, height: 9, borderRadius: 4.5, backgroundColor: "#ffffff" }} />
    </View>
  );
}

function Flow(ctx, { route, color, index }) {
  const reveal = interpolate(ctx.hold.progress, [0.1 + index * 0.12, 0.52 + index * 0.12], [0, 1], { easing: "easeInOut" });
  return <Path key="route" d={route} fill="none" stroke={color} strokeWidth="4.5" strokeLinecap="round" trimEnd={reveal} arrowEnd="triangle" arrowSize="16.5" style={{ opacity: 0.92 }} />;
}

function MapLabel(ctx, { region, placement, color, index }) {
  const reveal = interpolate(ctx.hold.progress, [0.22 + index * 0.035, 0.4 + index * 0.035], [0, 1]);
  return (
    <View key="label" className="absolute" style={{ borderStyle: "solid", left: placement.x, top: placement.y, height: 37.5, paddingLeft: 13.5, paddingRight: 13.5, borderRadius: 18, backgroundColor: "#020617c9", borderWidth: 1.5, borderColor: "#ffffff1f", opacity: placement.visible ? reveal : 0 }}>
      <Text key="text" style={{ marginTop: 6, fontFamily: "asset://brandFont", fontSize: 22.5, letterSpacing: 2.7, color }}>{region.name}</Text>
    </View>
  );
}

function WorldMap(ctx, { geometry, width, height, radius, gridOne, gridTwo, strokeWidth, ocean, land, accent }) {
  return (
    <View key="map" className="absolute" style={{ borderStyle: "solid", left: 0, top: 0, width, height, borderRadius: radius, backgroundColor: ocean, overflow: "hidden", borderWidth: 1.5, borderColor: accent }}>
      <View key="graticule-a" className="absolute" style={{ left: 0, top: gridOne, width, height: 1.5, backgroundColor: accent, opacity: 0.14 }} />
      <View key="graticule-b" className="absolute" style={{ left: 0, top: gridTwo, width, height: 1.5, backgroundColor: accent, opacity: 0.14 }} />
      {REGIONS.map((region, index) => <Region key={`region-${region.id}`} geometry={geometry[index]} fill={land} stroke={accent} strokeWidth={strokeWidth} index={index} />)}
    </View>
  );
}

export default function MapAtlas(ctx, props) {
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#030712", fontFamily: "asset://brandFont" }} camera={{ center: point(960, 540), zoom: 1, rotation: 0 }}>
      <World key="world">
        <View key="main-host" className="absolute" style={{ left: 105, top: 222, width: 1275, height: 765, filter: "drop-shadow(0px 26px 52px #00000099)" }}>
          <WorldMap key="main-map" geometry={MAIN} width={1275} height={765} radius={42} gridOne={255} gridTwo={510} strokeWidth="3" ocean={props.ocean} land={props.land} accent={props.accent} />
          <GeometryBatch key="density-samples" geometry="circle" positions={DENSITY} sizes={2.1} fills="#67e8f9" opacities={0.08} style={{ position: "absolute", left: 0, top: 0, width: 1275, height: 765 }} />
          {FLOWS.map((route, index) => <Flow key={`flow-${index}`} route={route} color={props.signal} index={index} />)}
          {REGIONS.map((region, index) => <MapLabel key={`label-${region.id}`} region={region} placement={LABELS[index]} color={props.label} index={index} />)}
          {REGIONS.slice(0, 5).map((region, index) => <Marker key={`marker-${region.id}`} pointValue={MAIN[index].centroid} color={props.accent} index={index} />)}
        </View>
        <View key="mini-host" className="absolute" style={{ left: 1425, top: 711, width: 375, height: 204, filter: "drop-shadow(0px 18px 32px #00000088)" }}>
          <WorldMap key="mini-map" geometry={MINI} width={375} height={204} radius={21} gridOne={67.5} gridTwo={136.5} strokeWidth="1.5" ocean="#160b24" land="#3b1c54" accent="#c084fc" />
        </View>
      </World>
      <Screen key="hud">
        <Text key="kicker" className="absolute" style={{ left: 105, top: 72, fontFamily: "asset://brandFont", fontSize: 24, letterSpacing: 6, color: props.accent }}>SYNTHETIC MOBILITY ATLAS / 01</Text>
        <Text key="title" className="absolute" style={{ left: 102, top: 114, width: 1230, fontFamily: "asset://brandFont", fontSize: 69, color: props.label }}>One projection. Replaceable parts.</Text>
        <View key="legend" className="absolute flex items-center" style={{ borderStyle: "solid", left: 1401, top: 105, width: 420, height: 69, borderRadius: 27, backgroundColor: "#020617dd", borderWidth: 1.5, borderColor: "#ffffff1c" }}>
          <View key="legend-line" style={{ marginLeft: 27, width: 69, height: 4.5, backgroundColor: props.signal }} />
          <Text key="legend-copy" style={{ marginLeft: 18, fontFamily: "asset://brandFont", fontSize: 22.5, color: props.label }}>PREPARED FLOW</Text>
        </View>
      </Screen>
    </Scene>
  );
}
