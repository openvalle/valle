export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "chart-integration";

export const controls = defineControls({
  assets: { brandFont: asset({ kind: "font", required: true }) },
});

const TITLE_METRICS = measureText("TWO THEMES / ONE CONTRACT", { fontFamily: "asset://brandFont", fontSize: 57 });
const VALUES = [26, 51, 43, 78];
const X = scaleBand({ range: [81, 705], count: VALUES.length, paddingInner: 0.24 });
const Y_A = scaleLinear({ domain: [0, 80], range: [525, 135] });
const Y_B = scaleLinear({ domain: [0, 100], range: [525, 135] });
const TOP_A = VALUES.map((value) => Y_A.map(value));
const TOP_B = VALUES.map((value, index) => Y_B.map(value + index * 4));
const DENSE = Array.from({ length: 1200 }, (_, index) => point(105 + (index % 60) * 12, 585 + Math.floor(index / 60) * 7.5));

function CardBar(ctx, { index, accent, progress }) {
  const top = TOP_A[index] + (TOP_B[index] - TOP_A[index]) * progress;
  return <View key="bar" className="absolute" style={{ left: X.band(index)[0], top, width: X.bandwidth(), height: 525 - top, backgroundColor: accent, opacity: 0.7 + index * 0.08 }} />;
}

function ChartCard(ctx, { accent, title }) {
  const p = ctx.hold.progress;
  return (
    <View key="card" className="absolute" style={{ borderStyle: "solid", width: 810, height: 660, borderRadius: 42, backgroundColor: "#0f172acc", borderWidth: 1.5, borderColor: accent, displacement: displacement(19, point(0.008, 0.006), 4.5 + p * 3, { octaves: 2, mode: "fractal" }) }}>
      <Text key="title" style={{ marginLeft: 51, marginTop: 42, fontFamily: "asset://brandFont", fontSize: 42, color: accent }}>{title}</Text>
      {VALUES.map((value, index) => <CardBar key={`bar-${index}`} index={index} accent={accent} progress={p} />)}
    </View>
  );
}

export default function ChartIntegration(ctx) {
  const p = ctx.hold.progress;
  const first = bounds("primary/card");
  const second = bounds("secondary/card");
  const ax = first.x + first.width * 0.5;
  const ay = first.y + first.height * 0.5;
  const bx = second.x + second.width * 0.5;
  const by = second.y + second.height * 0.5;
  return (
    <Scene
      key="scene"
      className="relative h-full w-full"
      style={{ backgroundColor: "#050816", fontFamily: "asset://brandFont" }}
      camera={{ center: point(960 + (bx - ax) * (p - 0.5) * 0.05, 540 + (by - ay) * (p - 0.5) * 0.05), zoom: 1, rotation: 0 }}
    >
      <World key="world">
        <GeometryBatch
          key="dense-markers"
          geometry="circle"
          positions={DENSE}
          sizes={3.6}
          fills="#f8fafc"
          opacities={0.14}
          style={{ position: "absolute", left: 0, top: 0, width: 1920, height: 1080, filter: "blur(0.45px)" }}
        />
        <View key="primary-host" className="absolute" style={{ left: 135, top: 225, width: 810, height: 660 }}>
          <ChartCard key="primary" accent="#22d3ee" title="CYAN / PREPARED A→B" />
        </View>
        <View key="secondary-host" className="absolute" style={{ left: 1050, top: 225, width: 810, height: 660 }}>
          <ChartCard key="secondary" accent="#a78bfa" title="VIOLET / SAME PARTS" />
        </View>
      </World>
      <Screen key="hud">
        <View key="panel" className="absolute" style={{ left: 51, top: 42, width: TITLE_METRICS.width + 66, height: 96, borderRadius: 27, backgroundColor: "#020617dd" }}>
          <Text key="title" style={{ marginLeft: 33, marginTop: 19.5, fontFamily: "asset://brandFont", fontSize: 57, color: "#f8fafc" }}>TWO THEMES / ONE CONTRACT</Text>
        </View>
      </Screen>
    </Scene>
  );
}
