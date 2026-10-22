// Route map: a pin ferries across an illustrated archipelago, the camera follows it from stop to
// stop and pulls back at the end to show the whole trip.
// Shows the Scene camera with World and Screen, motionPath/follow, autoRotate, pointAt, tangentAt
// and pathLength, a dashed route revealed with trimEnd under a moving strokeDashoffset, freezeTime
// for the stops, connect() leader lines from map stops to screen-space cards, Ellipse and Polyline.
// Render: valle motion render examples/07-route-map.motion.tsx -o 07-route-map.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

const SEA = "#7FD1F7";
const LAND = "#B8F2A0";
const SAND = "#FFE29A";
const ROUTE_RED = "#FF5964";
const PIN = "#6A4C93";
const INK = "#2E2A4F";
const AT_ORIGIN = { position: "absolute", left: 0, top: 0 };

// Map coordinates are the pixels of the whole map, which the camera shows at zoom 1 at the end.
const MAP_CENTER = [960, 540];

// An island coast: a wobbly ellipse through twelve points, joined by one smooth closed curve whose
// handles run parallel to the line between each point's neighbours. Built once, while the scene
// is prepared.
function coast(cx, cy, rx, ry, seed) {
  const count = 12;
  const points = Array.from({ length: count }, (_, i) => {
    const angle = (i / count) * TAU;
    const wobble = 1 + 0.12 * Math.sin(3 * angle + seed) + 0.07 * Math.sin(5 * angle + 2 * seed);
    return [cx + Math.cos(angle) * rx * wobble, cy + Math.sin(angle) * ry * wobble];
  });
  const at = (i) => points[(i + count) % count];
  const curves = points.map((p1, i) => {
    const p0 = at(i - 1);
    const p2 = at(i + 1);
    const p3 = at(i + 2);
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
    return `C ${c1.join(" ")} ${c2.join(" ")} ${p2.join(" ")}`;
  });
  return path(`M ${points[0].join(" ")} ${curves.join(" ")} Z`);
}

// Round treetops as one path, so a whole grove is a single shape.
function grove(trees) {
  return path(trees
    .map(([x, y, r]) => `M ${x - r} ${y} a ${r} ${r} 0 1 0 ${2 * r} 0 a ${r} ${r} 0 1 0 ${-2 * r} 0 Z`)
    .join(" "));
}

const ISLANDS = [
  {
    id: "harbor-isle", shape: coast(230, 850, 300, 210, 0.7), reef: coast(230, 850, 348, 256, 0.7),
    hills: coast(170, 880, 150, 100, 1.9),
    trees: grove([[120, 760, 22], [160, 800, 18], [88, 822, 16], [236, 712, 20], [300, 902, 24],
      [342, 944, 18], [262, 962, 16]]),
  },
  {
    id: "coral-isle", shape: coast(610, 290, 200, 140, 2.1), reef: coast(610, 290, 246, 184, 2.1),
    hills: coast(590, 270, 100, 64, 3.4),
    trees: grove([[560, 222, 20], [602, 250, 16], [520, 262, 15], [684, 240, 18]]),
  },
  {
    id: "light-isle", shape: coast(1210, 880, 240, 150, 4.2), reef: coast(1210, 880, 288, 196, 4.2),
    hills: coast(1220, 900, 120, 70, 0.4),
    trees: grove([[1080, 882, 20], [1120, 922, 16], [1342, 930, 18], [1384, 890, 14]]),
  },
  {
    id: "sunset-isle", shape: coast(1650, 300, 300, 220, 5.3), reef: coast(1650, 300, 348, 266, 5.3),
    hills: coast(1690, 260, 160, 110, 2.6),
    trees: grove([[1600, 160, 22], [1650, 202, 18], [1562, 222, 16], [1760, 180, 20], [1822, 262, 18],
      [1700, 382, 16]]),
  },
  {
    id: "rock-isle", shape: coast(1060, 290, 52, 36, 1.3), reef: coast(1060, 290, 92, 74, 1.3),
    hills: coast(1060, 290, 20, 14, 0.2), trees: grove([[1062, 284, 14]]),
  },
];

// Little "︵︵" swell marks scattered over open water.
const SWELLS = [
  [150, 170], [330, 96], [980, 92], [1240, 380], [880, 610], [620, 960], [1560, 980],
  [1820, 760], [1330, 620], [120, 470], [520, 560], [1700, 600], [1050, 700], [800, 1040],
];
const swells = (marks) => path(marks.map(([x, y]) => `M ${x} ${y} q 9 -9 18 0 q 9 -9 18 0`).join(" "));
const SWELLS_A = swells(SWELLS.filter((_, i) => i % 2 === 0));
const SWELLS_B = swells(SWELLS.filter((_, i) => i % 2 === 1));

// The trip, one cubic per leg: harbor → Coral Bay → Lighthouse Point → Sunset Cove.
const LEG_CURVES = [
  "C 560 680 720 520 752 372",
  "C 880 360 1080 600 1172 748",
  "C 1290 700 1380 560 1440 440",
];
const HARBOR = [455, 752];
// Each card is pinned to the screen by one edge, the side its leader line meets.
const STOPS = [
  { id: "coral", step: "1", name: "Coral Bay", note: "Snorkel the reef · 09:30", at: [752, 372],
    side: "left", edge: 860, top: 128 },
  { id: "lighthouse", step: "2", name: "Lighthouse Point", note: "120 steps up · 12:15", at: [1172, 748],
    side: "right", edge: 1060, top: 702 },
  { id: "sunset", step: "3", name: "Sunset Cove", note: "Golden hour · 18:40", at: [1440, 440],
    side: "left", edge: 1520, top: 592 },
];
const ROUTE = path(`M ${HARBOR.join(" ")} ${LEG_CURVES.join(" ")}`);
const LEGS = LEG_CURVES.map((curve, i) =>
  path(`M ${(i === 0 ? HARBOR : STOPS[i - 1].at).join(" ")} ${curve}`));

// Schedule in seconds: the pin leaves the harbor, takes under a second per leg and waits at the
// first two stops; the camera pulls back while it sails the last leg.
const DEPART = 0.45;
const LEG = 0.95;
const WAIT = 0.3;
const ARRIVE = [0, 1, 2].map((i) => DEPART + (i + 1) * LEG + i * WAIT);
// After the pin lands, the destination sends out two more ripples.
const FINALE = defineSequence({ beacon: stage({ at: ARRIVE[2] + 0.4, duration: 1.7 }) });
// The camera covers this share of the pin's offset from the map's centre, a moment behind it.
const FOLLOW = 0.55;
const CAMERA_LAG = 0.2;

// The trip clock runs without the stops; freezeTime holds it while the pin waits. Freezing the later
// stop first keeps both intervals in scene seconds.
function tripClock(seconds) {
  const pastSecondStop = freezeTime(seconds, { from: ARRIVE[1], to: ARRIVE[1] + WAIT });
  return freezeTime(pastSecondStop, { from: ARRIVE[0], to: ARRIVE[0] + WAIT });
}

// 0 before a leg, 1 after it, linear in trip time between.
function legPhase(clock, leg) {
  return clamp((clock - DEPART - leg * LEG) / LEG, 0, 1);
}

// The distance travelled along the route as a fraction of it: every leg eases out of one stop
// into the next, so the pin always arrives and leaves at rest.
function routeProgress(seconds) {
  const clock = tripClock(seconds);
  const travelled = (leg) =>
    interpolate(legPhase(clock, leg), [0, 1], [0, 1], { easing: "easeInOutSine" }) * pathLength(LEGS[leg]);
  return (travelled(0) + travelled(1) + travelled(2)) /
    (pathLength(LEGS[0]) + pathLength(LEGS[1]) + pathLength(LEGS[2]));
}

// How fast the pin moves, 0 at rest and 1 at the middle of a leg.
function cruise(seconds) {
  const clock = tripClock(seconds);
  const speed = (leg) => sin(deg(180) * legPhase(clock, leg));
  return speed(0) + speed(1) + speed(2);
}

// Painted bottom up: the dotted reef line, the island's raised edge, shallows, beach, land, trees.
function Island({ island }) {
  return (
    <Group key="island">
      <Path key="reef" style={AT_ORIGIN} d={island.reef} fill="none" stroke="#FFFFFFB3"
        strokeWidth={3} strokeDasharray="1 11" strokeLinecap="round" />
      <Path key="depth" style={{ ...AT_ORIGIN, translate: point(0, 16) }} d={island.shape} fill="#62BFEB"
        stroke="#62BFEB" strokeWidth={58} strokeLinejoin="round" />
      <Path key="shallows" style={AT_ORIGIN} d={island.shape} fill="none" stroke="#FFFFFF59"
        strokeWidth={58} strokeLinejoin="round" />
      <Path key="beach" style={AT_ORIGIN} d={island.shape} fill={SAND} stroke={SAND}
        strokeWidth={22} strokeLinejoin="round" />
      <Path key="land" style={AT_ORIGIN} d={island.shape} fill={LAND} />
      <Path key="hills" style={AT_ORIGIN} d={island.hills} fill="#A6EA8F" />
      <Path key="tree-shade" style={{ ...AT_ORIGIN, translate: point(4, 6) }} d={island.trees} fill="#5FBF6E66" />
      <Path key="trees" style={AT_ORIGIN} d={island.trees} fill="#7ED88A" />
    </Group>
  );
}

function Lighthouse() {
  const { seconds } = useCurrentTime();
  const glow = 0.5 + 0.5 * cos(seconds * TAU * 0.8);
  return (
    <View key="lighthouse" className="absolute" style={{ left: 1292, top: 800 }}>
      <Ellipse key="base" style={AT_ORIGIN} cx={0} cy={0} rx={22} ry={7} fill="#5FBF6E80" />
      <Polygon key="tower" style={AT_ORIGIN} fill="#FFFFFF"
        points={[point(-13, 0), point(13, 0), point(9, -64), point(-9, -64)]} />
      <Polygon key="stripe-low" style={AT_ORIGIN} fill={ROUTE_RED}
        points={[point(-12, -16), point(12, -16), point(11, -28), point(-11, -28)]} />
      <Polygon key="stripe-high" style={AT_ORIGIN} fill={ROUTE_RED}
        points={[point(-10.5, -40), point(10.5, -40), point(9.7, -52), point(-9.7, -52)]} />
      <Circle key="lamp-glow" style={AT_ORIGIN} cx={0} cy={-71} r={13 + 5 * glow}
        fill={interpolate(glow, [0, 1], ["#FFE29A00", "#FFE29AAA"])} />
      <Rect key="lamp" style={AT_ORIGIN} x={-8} y={-78} width={16} height={14} fill="#FFD23F" />
      <Polygon key="roof" style={AT_ORIGIN} points={[point(-12, -78), point(12, -78), point(0, -92)]} fill={INK} />
    </View>
  );
}

// A top-down boat that sails along its own course, its bow turned along the path.
function Boat({ course, start, speed, hull }) {
  const { seconds } = useCurrentTime();
  return (
    <View key="boat" className="absolute"
      style={{ motionPath: follow(course, start + speed * seconds, { rotate: "auto" }) }}>
      <Polyline key="wake" style={AT_ORIGIN} points={[point(-58, -13), point(-22, 0), point(-58, 13)]}
        fill="none" stroke="#FFFFFFB3" strokeWidth={4} strokeLinecap="round" strokeLinejoin="round" />
      <Path key="hull" style={AT_ORIGIN} fill={hull}
        d={path("M 24 0 C 14 -11 -14 -11 -22 -9 L -22 9 C -14 11 14 11 24 0 Z")} />
      <Rect key="cabin" style={AT_ORIGIN} x={-12} y={-5} width={16} height={10} fill="#FFFFFF" />
    </View>
  );
}

function StopMarker({ stop, index }) {
  const { seconds } = useCurrentTime();
  const arrival = ARRIVE[index];
  const arrived = interpolate(seconds, [arrival - 0.12, arrival + 0.12], [0, 1]);
  const pulse = sin(deg(180) * interpolate(seconds, [arrival - 0.1, arrival + 0.3], [0, 1], { easing: "easeOutCubic" }));
  const ripple = interpolate(seconds, [arrival, arrival + 0.9], [0, 1], { easing: "easeOutCubic" });
  const beacon = interpolate(repeatProgress(seconds, FINALE.beacon, 2), [0, 1], [0, 1], { easing: "easeOutCubic" });
  return (
    <View key="stop" className="absolute" style={{ left: stop.at[0], top: stop.at[1] }}>
      <Circle key="ripple" style={AT_ORIGIN} cx={0} cy={0} r={18 + 46 * ripple} fill="none"
        stroke={interpolate(ripple, [0, 0.15, 1], ["#FF596400", "#FF5964", "#FF596400"])} strokeWidth={4} />
      {index === STOPS.length - 1 && (
        <Circle key="beacon" style={AT_ORIGIN} cx={0} cy={0} r={18 + 40 * beacon} fill="none"
          stroke={interpolate(beacon, [0, 0.2, 1], ["#FF596400", "#FF5964CC", "#FF596400"])} strokeWidth={3} />
      )}
      <Circle key="dot" style={{ ...AT_ORIGIN, scale: 1 + 0.3 * pulse }} cx={0} cy={0} r={15}
        fill={interpolate(arrived, [0, 1], ["#FFFFFF", ROUTE_RED])}
        stroke={interpolate(arrived, [0, 1], [INK, "#FFFFFF"])} strokeWidth={5} />
    </View>
  );
}

// The pin floats higher the faster it moves, leans into its heading and bobs while it waits.
function Pin({ progress, cruising }) {
  const { seconds } = useCurrentTime();
  const appear = 1.2 * spring(seconds - 0.05, { duration: 0.5, bounce: 0.45 });
  const heading = tangentAt(ROUTE, progress);
  const lift = 16 * cruising + 3 * (1 - cruising) * sin(seconds * TAU * 0.9);
  const lean = clamp(heading.x * cruising * 9, -9, 9);
  return (
    <View key="pin" className="absolute" style={{ motionPath: motionPath(ROUTE, progress, { rotate: "none" }) }}>
      <Ellipse key="shadow" style={AT_ORIGIN} cx={0} cy={0}
        rx={0.01 + 14 * appear - 5 * cruising} ry={0.01 + 5 * appear - 2 * cruising}
        fill={interpolate(cruising, [0, 1], ["#2E2A4F4D", "#2E2A4F1F"])} />
      <Group key="body" style={{ transform: `translate(0px, ${-lift}px) rotate(${lean}deg) scale(${appear})` }}>
        <Path key="drop" style={AT_ORIGIN} fill={PIN}
          d={path("M 0 0 C -8 -12 -24 -26 -24 -46 A 24 24 0 1 1 24 -46 C 24 -26 8 -12 0 0 Z")} />
        <Path key="shade" style={AT_ORIGIN} fill="#57397D"
          d={path("M 0 0 C 8 -12 24 -26 24 -46 A 24 24 0 0 0 0 -70 L 0 0 Z")} />
        <Circle key="eye" style={AT_ORIGIN} cx={0} cy={-46} r={9.5} fill="#FFFFFF" />
      </Group>
    </View>
  );
}

function StopCard({ stop, index }) {
  const { seconds } = useCurrentTime();
  const { width } = useComposition();
  const shown = spring(seconds - ARRIVE[index] - 0.08, { duration: 0.55, bounce: 0.3 });
  // The badge sits on the side the leader line meets, so the card grows out of the line.
  const left = stop.side === "left";
  return (
    <View key="card" className="absolute flex items-center" style={{
      [stop.side]: left ? stop.edge : width - stop.edge, top: stop.top,
      flexDirection: left ? "row" : "row-reverse", gap: 18,
      padding: left ? "16px 30px 16px 16px" : "16px 16px 16px 30px", borderRadius: 26,
      backgroundColor: "#FFFFFF", boxShadow: "0px 7px 0px #2E2A4F29",
      transformOrigin: left ? "0% 50%" : "100% 50%", scale: 0.7 + 0.3 * shown,
      opacity: clamp(shown * 2.5, 0, 1) }}>
      <View key="badge" className="flex items-center justify-center"
        style={{ width: 54, height: 54, borderRadius: 27, backgroundColor: PIN }}>
        <Text key="step" style={{ fontSize: 28, fontWeight: 800, color: "#FFFFFF" }}>{stop.step}</Text>
      </View>
      <View key="copy" className="flex flex-col" style={{ gap: 2, alignItems: left ? "flex-start" : "flex-end" }}>
        <Text key="name" style={{ fontSize: 34, fontWeight: 800, color: INK }}>{stop.name}</Text>
        <Text key="note" style={{ fontSize: 22, fontWeight: 600, color: "#6E6A93" }}>{stop.note}</Text>
      </View>
    </View>
  );
}

export default function RouteMap() {
  const { seconds } = useCurrentTime();
  const { width, height } = useComposition();
  const progress = routeProgress(seconds);
  const cruising = cruise(seconds);

  // The camera trails the pin, then pulls back to the whole map while the pin sails the last leg.
  const trailing = pointAt(ROUTE, routeProgress(seconds - CAMERA_LAG));
  const pull = interpolate(seconds, [3.6, 5], [0, 1], { easing: "easeInOutSine" });
  const reach = FOLLOW * (1 - pull);
  const centerX = MAP_CENTER[0] + (trailing.x - MAP_CENTER[0]) * reach;
  const centerY = MAP_CENTER[1] + (trailing.y - MAP_CENTER[1]) * reach;
  const zoom = interpolate(seconds, [0, 1.1, 3.6, 5], [1.65, 1.4, 1.4, 1],
    { easing: ["easeInOutSine", "linear", "easeInOutSine"] });
  // Where a map point lands on screen under this camera.
  const onScreen = (at) => point(width / 2 + (at[0] - centerX) * zoom, height / 2 + (at[1] - centerY) * zoom);

  const intro = interpolate(seconds, [0.1, 0.7], [0, 1], { easing: "easeOutCubic" });
  const bob = 4 * sin(seconds * TAU * 0.5);

  return (
    <Scene className="relative h-full w-full"
      style={{ background: `radial-gradient(ellipse at 50% 45%, #8FDAFA 0%, ${SEA} 55%, #66C3EF 100%)` }}
      camera={{ center: point(centerX, centerY), zoom, rotation: 0 }}>
      <World>
        <Path key="swells-a" style={{ ...AT_ORIGIN, translate: point(0, bob) }} d={SWELLS_A} fill="none"
          stroke="#FFFFFF99" strokeWidth={4} strokeLinecap="round" strokeLinejoin="round" />
        <Path key="swells-b" style={{ ...AT_ORIGIN, translate: point(0, -bob) }} d={SWELLS_B} fill="none"
          stroke="#FFFFFF99" strokeWidth={4} strokeLinecap="round" strokeLinejoin="round" />
        {ISLANDS.map((island) => <Island key={island.id} island={island} />)}
        <Lighthouse />
        <Boat key="boat-west" course={path("M 90 470 C 190 410 300 420 390 486")}
          start={0.12} speed={0.11} hull={ROUTE_RED} />
        <Boat key="boat-east" course={path("M 1890 910 C 1800 860 1700 860 1610 900")}
          start={0.1} speed={0.12} hull={PIN} />
        {/* The pin's wake trails behind it on the water, turned along the route. */}
        <View key="wake" className="absolute"
          style={{ translate: pointAt(ROUTE, progress), rotate: autoRotate(ROUTE, progress) }}>
          <Polyline key="wake-line" style={AT_ORIGIN} fill="none"
            points={[point(-72, -21), point(-42, -10), point(-16, 0), point(-42, 10), point(-72, 21)]}
            stroke={interpolate(cruising, [0, 1], ["#FFFFFF00", "#FFFFFFCC"])} strokeWidth={4.5}
            strokeLinecap="round" strokeLinejoin="round" />
        </View>
        <Path key="route-casing" style={AT_ORIGIN} d={ROUTE} fill="none" stroke="#FFFFFF" strokeWidth={17}
          strokeLinecap="round" strokeDasharray="22 18" strokeDashoffset={-34 * seconds} trimEnd={progress} />
        <Path key="route" style={AT_ORIGIN} d={ROUTE} fill="none" stroke={ROUTE_RED} strokeWidth={9}
          strokeLinecap="round" strokeDasharray="22 18" strokeDashoffset={-34 * seconds} trimEnd={progress} />
        <View key="harbor" className="absolute" style={{ left: HARBOR[0], top: HARBOR[1] }}>
          <Circle key="harbor-base" style={AT_ORIGIN} cx={0} cy={0} r={8} fill={INK} />
          <Line key="harbor-pole" style={AT_ORIGIN} x1={0} y1={0} x2={0} y2={-40} fill="none" stroke={INK}
            strokeWidth={4} strokeLinecap="round" />
          <Polygon key="harbor-flag" style={{ ...AT_ORIGIN, transformOrigin: point(0, -40),
            rotate: `${wiggle(seconds, { seed: 7, frequency: 1.4, amplitude: 6 })}deg` }}
            points={[point(0, -40), point(30, -32), point(0, -24)]} fill={ROUTE_RED} />
        </View>
        {STOPS.map((stop, index) => <StopMarker key={stop.id} stop={stop} index={index} />)}
        <Pin progress={progress} cruising={cruising} />
      </World>
      <Screen>
        {/* Leader lines run from a stop on the moving map to its card on the screen. A component
            scopes the keys inside it, so the card of the "coral" StopCard is "coral/card". */}
        {STOPS.map((stop, index) => (
          <Path key={`${stop.id}-link`} style={AT_ORIGIN} fill="none" stroke={INK} strokeWidth={3}
            strokeLinecap="round"
            trimEnd={interpolate(seconds, [ARRIVE[index] - 0.1, ARRIVE[index] + 0.25], [0, 1], { easing: "easeOutCubic" })}
            d={connect(onScreen(stop.at), anchor(`${stop.id}/card`, stop.side), { route: "cubic" })} />
        ))}
        {STOPS.map((stop, index) => <StopCard key={stop.id} stop={stop} index={index} />)}
        <View key="title" className="absolute flex flex-col" style={{ left: 96, top: 72, padding: "20px 34px 24px 30px",
          borderRadius: 28, backgroundColor: "#FFFFFF", boxShadow: "0px 8px 0px #2E2A4F29",
          translate: point(-30 * (1 - intro), 0), opacity: intro }}>
          <Text key="kicker" style={{ fontSize: 20, fontWeight: 800, letterSpacing: 4, color: ROUTE_RED }}>
            DAY TRIP · 3 STOPS
          </Text>
          <Text key="heading" style={{ fontSize: 64, lineHeight: 1.05, color: INK,
            fontVariationSettings: '"wght" 860, "wdth" 78' }}>Island Hop</Text>
        </View>
        <View key="scale" className="absolute flex items-center" style={{ left: 96, top: 974, height: 50,
          gap: 16, padding: "0px 22px", borderRadius: 25, backgroundColor: "#FFFFFF",
          translate: point(-30 * (1 - intro), 0), opacity: intro }}>
          <View key="scale-art" className="relative" style={{ width: 150, height: 16 }}>
            <Rect key="scale-fill" style={AT_ORIGIN} x={0} y={6} width={75} height={8} fill={INK} />
            <Polyline key="scale-frame" style={AT_ORIGIN} fill="none" stroke={INK} strokeWidth={3}
              points={[point(0, 2), point(0, 14), point(150, 14), point(150, 2)]}
              strokeLinecap="round" strokeLinejoin="round" />
          </View>
          <Text key="scale-label" style={{ fontSize: 20, fontWeight: 700, color: INK }}>10 km</Text>
        </View>
        <View key="compass" className="absolute" style={{ left: 1726, top: 64, width: 110, height: 110,
          borderRadius: 55, backgroundColor: "#FFFFFF", boxShadow: "0px 7px 0px #2E2A4F29", opacity: intro }}>
          <View key="needle" className="absolute" style={{ left: 0, top: 0, width: 110, height: 110,
            rotate: `${-50 * (1 - spring(seconds - 0.2, { preset: "wobbly" }))}deg` }}>
            <Polygon key="north" style={AT_ORIGIN} fill={ROUTE_RED}
              points={[point(55, 16), point(66, 55), point(44, 55)]} />
            <Polygon key="south" style={AT_ORIGIN} fill={INK}
              points={[point(55, 94), point(66, 55), point(44, 55)]} />
            <Circle key="hub" style={AT_ORIGIN} cx={55} cy={55} r={6} fill="#FFFFFF" />
          </View>
        </View>
      </Screen>
    </Scene>
  );
}
