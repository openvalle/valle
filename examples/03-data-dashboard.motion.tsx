// Data dashboard: a flat infographic that builds itself. KPIs count up, stacked bars rise in a
// stagger, a line draws as its area pours in, a donut opens and the record month is called out.
// Shows d3's scaleLinear/scaleBand/nice/ticks/stack/line/area/pie/arc in inline SVG, `.map`
// instances, defineSequence + staggerProgress, Grid/Flex with Tailwind, formatNumber/formatPercent
// counters, an area uncovered behind the line head, motionBlur on the head and a steps() blink.
// Render: valle motion render examples/03-data-dashboard.motion.tsx -o 03-data-dashboard.mp4

export const composition = { width: 1920, height: 1080, fps: 60, duration: 6 };

// Easings.
const IN_OUT_CUBIC = "cubic-bezier(0.65, 0, 0.35, 1)";
const OUT_BACK = "cubic-bezier(0.34, 1.56, 0.64, 1)";
const OUT_CUBIC = "cubic-bezier(0.33, 1, 0.68, 1)";
const OUT_EXPO = "cubic-bezier(0.16, 1, 0.3, 1)";

const BG = "#EEF4FF";
const INK = "#1A237E";
const MUTED = "#5C6BC0";
const BLUE = "#3D5AFE";
const CYAN = "#00B8D4";
const ORANGE = "#FF6D00";
const AMBER = "#FFC400";
const GRID = "#E3E9FD";
const VALUE = { fontSize: 56, fontWeight: 800, color: INK, fontVariantNumeric: "tabular-nums" };

const SEQ = defineSequence({
  header: { duration: 0.8 },
  tiles: { at: 0.2, duration: 0.7 },
  cards: { after: "tiles", overlap: 0.4, duration: 0.8 },
  count: { at: 0.45, duration: 1.6 },
  grid: { at: 1.15, duration: 0.6 },
  bars: { at: 1.35, duration: 0.9 },
  line: { at: 1.7, duration: 1.9 },
  donut: { at: 2.1, duration: 1.2 },
  focus: { at: 3.9, duration: 0.6 },
});

// Monthly revenue in $k, two stacked series.
const MONTHS = ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];
const SUBSCRIPTIONS = [182, 196, 214, 229, 241, 262, 281, 298, 322, 341, 366, 392];
const SERVICES = [64, 71, 69, 78, 86, 84, 95, 102, 99, 113, 121, 130];
const ROWS = MONTHS.map((_, i) => ({ subs: SUBSCRIPTIONS[i], services: SERVICES[i] }));
const LAYERS = stack().keys(["subs", "services"])(ROWS);
const TOTALS = ROWS.map((r) => r.subs + r.services);
const BAR_X = scaleBand()
  .domain(MONTHS.map((_, i) => i))
  .range([96, 664])
  .paddingInner(0.34)
  .paddingOuter(0.2);
const BAR_Y = scaleLinear()
  .domain([0, Math.max(...TOTALS)])
  .range([548, 140])
  .nice(4);
const BASE = BAR_Y(0);
const RECORD = MONTHS.length - 1;
const SERIES = [
  { color: BLUE, dim: "#94A4FE" },
  { color: CYAN, dim: "#73D8E7" },
];
// One row per stacked segment, in pixels above the baseline. Each month's stack fills like one
// bar: a segment shows the part of the rising total that lies inside it, and only the top
// segment may overshoot. Lower segments reach 1px under the next one, so no seam shows.
const SEGMENTS = LAYERS.flatMap((layer, series) =>
  layer.map(([low, high], month) => ({
    id: `bar-${series}-${month}`,
    month,
    x: BAR_X(month),
    low: BASE - BAR_Y(low),
    total: BASE - BAR_Y(TOTALS[month]),
    cap: series === SERIES.length - 1 ? 1000 : BAR_Y(low) - BAR_Y(high) + 1,
    color: SERIES[series].color,
    dim: month === RECORD ? SERIES[series].color : SERIES[series].dim,
  })),
);
const GRIDLINES = BAR_Y.ticks(4).map((value, index) => ({
  id: `grid-${index}`,
  index,
  y: BAR_Y(value),
  label: value === 0 ? "$0" : `$${value}k`,
}));
const MONTH_LABELS = MONTHS.map((label, index) => ({ id: `month-${index}`, index, label }));
const RECORD_TOP = BAR_Y(TOTALS[RECORD]);

// Weekly active users in thousands.
const WAU = [42, 45, 44, 49, 53, 51, 58, 62, 60, 67, 71, 69, 76, 82, 85, 91];
const PLOT = { left: 76, right: 596, top: 170, bottom: 540 };
const LINE_X = scaleLinear()
  .domain([0, WAU.length - 1])
  .range([PLOT.left, PLOT.right]);
const LINE_Y = scaleLinear()
  .domain([Math.min(...WAU) - 12, Math.max(...WAU)])
  .range([PLOT.bottom, PLOT.top])
  .nice(4);
const USERS = LINE_Y.domain();
const WAU_POINTS = WAU.map((value, index) => ({ x: LINE_X(index), y: LINE_Y(value) }));
const TREND = line()
  .x((p) => p.x)
  .y((p) => p.y)
  .curve(curveMonotoneX)(WAU_POINTS);
const TREND_AREA = area()
  .x((p) => p.x)
  .y0(LINE_Y(USERS[0]))
  .y1((p) => p.y)
  .curve(curveMonotoneX)(WAU_POINTS);
const USER_TICKS = LINE_Y.ticks(4).map((value, index) => ({
  id: `users-${index}`,
  index,
  y: LINE_Y(value),
  label: `${value}k`,
}));

// When the drawing head passes each point: its share of the polyline's length, mapped back
// through the line's ease-in-out cubic by bisection.
function cumulative(values) {
  const sums = [];
  let total = 0;
  for (const value of values) {
    total += value;
    sums.push(total);
  }
  return sums;
}
function easeInOutCubicInverse(y) {
  let low = 0;
  let high = 1;
  for (let step = 0; step < 40; step += 1) {
    const mid = (low + high) / 2;
    const rest = 2 - 2 * mid;
    const eased = mid < 0.5 ? 4 * mid * mid * mid : 1 - (rest * rest * rest) / 2;
    if (eased < y) low = mid;
    else high = mid;
  }
  return (low + high) / 2;
}
const TRAVEL = cumulative(
  WAU_POINTS.map((p, i) => {
    if (i === 0) return 0;
    const dx = p.x - WAU_POINTS[i - 1].x;
    const dy = p.y - WAU_POINTS[i - 1].y;
    return Math.sqrt(dx * dx + dy * dy);
  }),
);
const DOTS = WAU_POINTS.map((p, i) => ({
  id: `dot-${i}`,
  x: p.x,
  y: p.y,
  at:
    SEQ.line.start +
    SEQ.line.duration * easeInOutCubicInverse(TRAVEL[i] / TRAVEL[TRAVEL.length - 1]),
}));
const LAST = WAU_POINTS[WAU_POINTS.length - 1];
const LINE_END = SEQ.line.start + SEQ.line.duration;

// Revenue mix by channel, the ring from 12 o'clock round clockwise.
const MIX = [
  { id: "direct", label: "Direct", value: 0.42, color: BLUE },
  { id: "partners", label: "Partners", value: 0.26, color: CYAN },
  { id: "market", label: "Marketplace", value: 0.18, color: AMBER },
  { id: "other", label: "Other", value: 0.14, color: ORANGE },
];
const SLICES = pie()
  .value((item) => item.value)
  .sort(null)
  .padAngle((2.4 * Math.PI) / 180)(MIX);
const DONUT = { x: 198, y: 270, inner: 80, outer: 126 };

function ease(progress) {
  return interpolate(progress, [0, 1], [0, 1], { easing: OUT_CUBIC });
}
function segmentHeight(segment, seconds) {
  const rise = interpolate(
    staggerProgress(seconds, SEQ.bars, segment.month, 0.06),
    [0, 1],
    [0, 1],
    { easing: "cubic-bezier(0.22, 1.25, 0.36, 1)" },
  );
  return clamp(segment.total * rise - segment.low, 0, segment.cap);
}
function counted(seconds, index) {
  return interpolate(staggerProgress(seconds, SEQ.count, index, 0.12), [0, 1], [0, 1], {
    easing: OUT_EXPO,
  });
}

function Tile({ index, fill = "#FFFFFF", children }) {
  const { seconds } = useCurrentTime();
  const rise = spring(seconds - SEQ.tiles.start - index * 0.07, { duration: 0.6, bounce: 0.25 });
  return (
    <View
      className="flex flex-col justify-between rounded-3xl p-7"
      style={{
        backgroundColor: fill,
        opacity: clamp(rise * 1.6, 0, 1),
        translate: `0px ${32 * (1 - rise)}px`,
      }}
    >
      {children}
    </View>
  );
}

function KpiTile({ index, label, delta, accent, children }) {
  return (
    <Tile index={index}>
      <View className="flex items-center justify-between">
        <View className="flex items-center gap-3">
          <View style={{ width: 14, height: 14, borderRadius: 7, backgroundColor: accent }} />
          <Text style={{ fontSize: 19, fontWeight: 700, letterSpacing: 2, color: MUTED }}>
            {label}
          </Text>
        </View>
        <View className="rounded-full px-3 py-1" style={{ backgroundColor: `${accent}1f` }}>
          <Text style={{ fontSize: 18, fontWeight: 700, color: accent }}>{delta}</Text>
        </View>
      </View>
      {children}
    </Tile>
  );
}

function ChartCard({ index, title, subtitle, children }) {
  const { seconds } = useCurrentTime();
  const rise = ease(staggerProgress(seconds, SEQ.cards, index, 0.08));
  return (
    <View
      className="relative rounded-3xl bg-white"
      style={{ opacity: clamp(rise * 1.5, 0, 1), translate: `0px ${36 * (1 - rise)}px` }}
    >
      <Text
        className="absolute"
        style={{ left: 36, top: 30, fontSize: 26, fontWeight: 800, color: INK }}
      >
        {title}
      </Text>
      <Text className="absolute" style={{ left: 36, top: 68, fontSize: 17, color: MUTED }}>
        {subtitle}
      </Text>
      {children}
    </View>
  );
}

function Legend({ items }) {
  return (
    <View className="flex items-center gap-5">
      {items.map((item) => (
        <View key={item.id} className="flex items-center gap-2">
          <View style={{ width: 14, height: 14, borderRadius: 4, backgroundColor: item.color }} />
          <Text style={{ fontSize: 16, fontWeight: 600, color: MUTED }}>{item.label}</Text>
        </View>
      ))}
    </View>
  );
}

// One clockwise sweep opens the ring; each slice shows the part of the sweep that lies inside it.
function donutSweep(seconds) {
  return (
    2 *
    Math.PI *
    interpolate(stageProgress(seconds, SEQ.donut), [0, 1], [0, 1], { easing: IN_OUT_CUBIC })
  );
}

function Slice({ index, color, lift }) {
  const { seconds } = useCurrentTime();
  const slice = SLICES[index];
  const end = clamp(donutSweep(seconds), slice.startAngle, slice.endAngle);
  if (end <= slice.startAngle + 0.01) return null;
  return (
    <path
      fill={color}
      transform={`translate(${DONUT.x} ${DONUT.y})`}
      d={arc()
        .innerRadius(DONUT.inner)
        .outerRadius(DONUT.outer + lift)
        .cornerRadius(6)
        .padAngle(slice.padAngle)
        .startAngle(slice.startAngle)
        .endAngle(end)()}
    />
  );
}

export default function DataDashboard() {
  const { seconds } = useCurrentTime();
  const header = ease(stageProgress(seconds, SEQ.header));
  const draw = interpolate(stageProgress(seconds, SEQ.line), [0, 1], [0, 1], {
    easing: IN_OUT_CUBIC,
  });
  const head = pointAt(TREND, draw);
  const focus = ease(stageProgress(seconds, SEQ.focus));
  const pop = spring(seconds - SEQ.focus.start - 0.1, { duration: 0.5, bounce: 0.35 });
  const pulse = (seconds - LINE_END) / 1.2 - Math.floor((seconds - LINE_END) / 1.2);
  // Blinks twice a second; steps() holds each half of the cycle.
  const live = interpolate((seconds + 0.25) % 1, [0, 1], [ORANGE, "#FF6D0033"], {
    easing: "steps(2, jump-none)",
  });

  return (
    <View className="h-full w-full flex flex-col gap-7 p-16" style={{ backgroundColor: BG }}>
      <View
        className="flex h-24 items-center justify-between"
        style={{ opacity: header, translate: `0px ${24 * (1 - header)}px` }}
      >
        <View className="flex flex-col gap-1">
          <Text style={{ fontSize: 20, fontWeight: 700, letterSpacing: 4, color: BLUE }}>
            Q3 2026 · GROWTH REPORT
          </Text>
          <Text style={{ fontSize: 56, fontWeight: 800, color: INK }}>
            {"Everything is "}
            <Span style={{ color: BLUE }}>trending up</Span>
          </Text>
        </View>
        <View className="flex items-center gap-3 rounded-full bg-white px-6 py-3">
          <View style={{ width: 14, height: 14, borderRadius: 7, backgroundColor: live }} />
          <Text style={{ fontSize: 18, fontWeight: 800, letterSpacing: 3, color: INK }}>LIVE</Text>
          <Text style={{ fontSize: 18, color: MUTED }}>updated just now</Text>
        </View>
      </View>

      <View className="grid grid-cols-4 gap-7" style={{ height: 168 }}>
        <KpiTile key="revenue" index={0} label="REVENUE" delta="+18.2%" accent={BLUE}>
          <Text style={VALUE}>${formatNumber(4.82 * counted(seconds, 0), { decimals: 2 })}M</Text>
        </KpiTile>
        <KpiTile key="users" index={1} label="ACTIVE USERS" delta="+9.6%" accent={CYAN}>
          <Text style={VALUE}>{formatNumber(128400 * counted(seconds, 1), { decimals: 0 })}</Text>
        </KpiTile>
        <KpiTile key="conversion" index={2} label="CONVERSION" delta="+1.2 pts" accent={ORANGE}>
          <Text style={VALUE}>{formatPercent(0.068 * counted(seconds, 2), { decimals: 1 })}</Text>
        </KpiTile>
        <Tile key="growth" index={3} fill={AMBER}>
          <Text style={{ fontSize: 19, fontWeight: 700, letterSpacing: 2, color: INK }}>
            QUARTER GROWTH
          </Text>
          <View className="flex items-end justify-between">
            <View className="flex items-center gap-3" style={{ color: INK }}>
              <Text style={{ fontSize: 30, fontStyle: "italic" }}>g =</Text>
              <View className="flex flex-col items-center">
                <Text style={{ fontSize: 24 }}>
                  R<Span style={{ fontSize: 15 }}>Q3</Span> − R
                  <Span style={{ fontSize: 15 }}>Q2</Span>
                </Text>
                <View style={{ alignSelf: "stretch", height: 2, backgroundColor: INK }} />
                <Text style={{ fontSize: 24 }}>
                  R<Span style={{ fontSize: 15 }}>Q2</Span>
                </Text>
              </View>
            </View>
            <Text style={VALUE}>
              +{formatPercent(0.246 * counted(seconds, 3), { decimals: 1 })}
            </Text>
          </View>
        </Tile>
      </View>

      <View className="grid flex-1 grid-cols-[700px_640px_1fr] grid-rows-1 gap-7">
        <ChartCard
          key="revenue-card"
          index={0}
          title="Monthly revenue"
          subtitle="Thousands of dollars, 2026"
        >
          <View className="absolute" style={{ left: 380, top: 38 }}>
            <Legend
              items={[
                { id: "subs", label: "Subscriptions", color: BLUE },
                { id: "services", label: "Services", color: CYAN },
              ]}
            />
          </View>
          {GRIDLINES.map((line) => (
            <View
              key={line.id}
              className="absolute"
              style={{
                left: 96,
                top: line.y - 1,
                height: 2,
                backgroundColor: GRID,
                width: 568 * ease(staggerProgress(seconds, SEQ.grid, line.index, 0.06)),
              }}
            />
          ))}
          {GRIDLINES.map((line) => (
            <Text
              key={`${line.id}-label`}
              className="absolute"
              style={{
                left: 16,
                top: line.y - 12,
                width: 66,
                fontSize: 16,
                textAlign: "right",
                color: MUTED,
                opacity: ease(staggerProgress(seconds, SEQ.grid, line.index, 0.06)),
              }}
            >
              {line.label}
            </Text>
          ))}
          {SEGMENTS.map((segment) => (
            <View
              key={segment.id}
              className="absolute"
              style={{
                left: segment.x,
                width: BAR_X.bandwidth(),
                top: BASE - segment.low - segmentHeight(segment, seconds),
                height: segmentHeight(segment, seconds),
                backgroundColor: interpolate(focus, [0, 1], [segment.color, segment.dim]),
              }}
            />
          ))}
          <View
            className="absolute flex"
            style={{ left: BAR_X(0) + BAR_X.bandwidth() / 2 - BAR_X.step() / 2, top: 562 }}
          >
            {MONTH_LABELS.map((month) => (
              <Text
                key={month.id}
                style={{
                  width: BAR_X.step(),
                  fontSize: 17,
                  fontWeight: 600,
                  textAlign: "center",
                  color: interpolate(
                    staggerProgress(seconds, SEQ.bars, month.index, 0.06),
                    [0, 1],
                    ["#5C6BC000", MUTED],
                  ),
                }}
              >
                {month.label}
              </Text>
            ))}
          </View>
          <View
            className="absolute flex flex-col items-end"
            style={{
              left: BAR_X(RECORD) + BAR_X.bandwidth() - 220,
              top: RECORD_TOP - 92,
              width: 220,
              opacity: clamp(pop * 2, 0, 1),
              scale: 0.6 + 0.4 * pop,
              transformOrigin: `${220 - BAR_X.bandwidth() / 2}px 100%`,
            }}
          >
            <View className="rounded-full px-4 py-2" style={{ backgroundColor: INK }}>
              <Text style={{ fontSize: 17, fontWeight: 700, color: "#FFFFFF" }}>
                Record · $522k
              </Text>
            </View>
            <View
              style={{
                width: 2,
                height: 30,
                marginRight: BAR_X.bandwidth() / 2 - 1,
                backgroundColor: INK,
              }}
            />
          </View>
        </ChartCard>

        <ChartCard
          key="users-card"
          index={1}
          title="Weekly active users"
          subtitle="Thousands, last 16 weeks"
        >
          {USER_TICKS.map((tick) => (
            <View
              key={tick.id}
              className="absolute"
              style={{
                left: PLOT.left,
                top: tick.y - 1,
                height: 2,
                backgroundColor: GRID,
                width:
                  (PLOT.right - PLOT.left) *
                  ease(staggerProgress(seconds, SEQ.grid, tick.index, 0.06)),
              }}
            />
          ))}
          {USER_TICKS.map((tick) => (
            <Text
              key={`${tick.id}-label`}
              className="absolute"
              style={{
                left: 8,
                top: tick.y - 12,
                width: 54,
                fontSize: 16,
                textAlign: "right",
                color: MUTED,
                opacity: ease(staggerProgress(seconds, SEQ.grid, tick.index, 0.06)),
              }}
            >
              {tick.label}
            </Text>
          ))}
          {["JUL", "AUG", "SEP", "OCT"].map((label, index) => (
            <Text
              key={label}
              className="absolute"
              style={{
                left: LINE_X(index * 5) - 40,
                top: 556,
                width: 80,
                fontSize: 16,
                fontWeight: 600,
                textAlign: "center",
                color: MUTED,
              }}
            >
              {label}
            </Text>
          ))}
          {/* The area pours in behind the head: its box is cut at the head's x. */}
          <View
            className="absolute overflow-hidden"
            style={{ left: 0, top: 0, height: 632, width: Math.max(head.x, PLOT.left + 1) }}
          >
            <svg className="absolute" width={640} height={632} style={{ left: 0, top: 0 }}>
              <defs>
                <linearGradient
                  id="pour"
                  gradientUnits="userSpaceOnUse"
                  x1={0}
                  y1={PLOT.top}
                  x2={0}
                  y2={PLOT.bottom}
                >
                  <stop offset={0} stopColor="#FF6D0059" />
                  <stop offset={1} stopColor="#FF6D0000" />
                </linearGradient>
              </defs>
              <path d={TREND_AREA} fill="url(#pour)" />
            </svg>
          </View>
          <svg className="absolute inset-0" width={640} height={632}>
            {/* Not drawn at all before it starts: a dash of no length still shows its round caps. */}
            {draw > 0 && (
              <path
                d={TREND}
                fill="none"
                stroke={ORANGE}
                strokeWidth={6}
                strokeLinecap="round"
                strokeLinejoin="round"
                pathLength={1}
                strokeDasharray="1 1"
                strokeDashoffset={1 - draw}
              />
            )}
            {DOTS.map((dot) => (
              <circle
                key={dot.id}
                cx={dot.x}
                cy={dot.y}
                fill={ORANGE}
                r={7 * interpolate(seconds - dot.at, [0, 0.3], [0, 1], { easing: OUT_BACK })}
              />
            ))}
            {DOTS.map((dot) => (
              <circle
                key={`${dot.id}-core`}
                cx={dot.x}
                cy={dot.y}
                fill="#FFFFFF"
                r={3 * interpolate(seconds - dot.at, [0.05, 0.3], [0, 1], { easing: OUT_CUBIC })}
              />
            ))}
          </svg>
          {/* The head runs as a translation, so motionBlur "auto" smears it along the line. */}
          {draw > 0 && (
            <View
              className="absolute"
              style={{
                left: -30,
                top: -30,
                width: 60,
                height: 60,
                translate: `${head.x}px ${head.y}px`,
                motionBlur: "auto",
              }}
            >
              <svg width={60} height={60}>
                {seconds >= LINE_END && (
                  <circle
                    cx={30}
                    cy={30}
                    r={11 + 18 * pulse}
                    fill="none"
                    strokeWidth={3}
                    stroke={interpolate(pulse, [0, 1], ["#FF6D0099", "#FF6D0000"])}
                  />
                )}
                <circle cx={30} cy={30} r={11} fill={ORANGE} />
                <circle cx={30} cy={30} r={4.5} fill="#FFFFFF" />
              </svg>
            </View>
          )}
          <View
            className="absolute rounded-full px-4 py-2"
            style={{
              left: LAST.x - 128,
              top: LAST.y - 62,
              backgroundColor: INK,
              opacity: clamp(pop * 2, 0, 1),
              scale: 0.6 + 0.4 * pop,
              transformOrigin: "100% 100%",
            }}
          >
            <Text style={{ fontSize: 17, fontWeight: 700, color: "#FFFFFF" }}>91k this week</Text>
          </View>
        </ChartCard>

        <ChartCard key="mix-card" index={2} title="Revenue mix" subtitle="Share by channel">
          <svg className="absolute inset-0" width={500} height={632}>
            {MIX.map((item, index) => (
              <Slice
                key={item.id}
                index={index}
                color={item.color}
                lift={index === 0 ? 10 * focus : 0}
              />
            ))}
          </svg>
          <View
            className="absolute flex flex-col items-center"
            style={{
              left: DONUT.x - 80,
              top: DONUT.y - 46,
              width: 160,
              opacity: clamp(pop * 2, 0, 1),
              scale: 0.7 + 0.3 * pop,
            }}
          >
            <Text style={{ fontSize: 50, fontWeight: 800, color: INK }}>42%</Text>
            <Text style={{ fontSize: 16, fontWeight: 700, letterSpacing: 2, color: MUTED }}>
              DIRECT
            </Text>
          </View>
          <View className="absolute flex flex-col gap-3" style={{ left: 36, top: 438, width: 324 }}>
            {MIX.map((item, index) => (
              <View
                key={`${item.id}-legend`}
                className="flex items-center justify-between"
                style={{
                  opacity: interpolate(
                    donutSweep(seconds) - SLICES[index].startAngle,
                    [0, 0.6],
                    [0, 1],
                  ),
                }}
              >
                <View className="flex items-center gap-3">
                  <View
                    style={{ width: 14, height: 14, borderRadius: 4, backgroundColor: item.color }}
                  />
                  <Text style={{ fontSize: 18, fontWeight: 600, color: INK }}>{item.label}</Text>
                </View>
                <Text style={{ fontSize: 18, fontWeight: 700, color: MUTED }}>
                  {formatPercent(item.value)}
                </Text>
              </View>
            ))}
          </View>
        </ChartCard>
      </View>
    </View>
  );
}
