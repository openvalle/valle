export const component = "chart-integration";

export const controls = defineControls({
  assets: { brandFont: asset({ kind: "font", required: true }) },
});

const TITLE_METRICS = measureText("TWO THEMES / ONE CONTRACT", { style: "font-family: 'asset://brandFont'; font-size: 38px" });
const VALUES = [26, 51, 43, 78];
const X = scaleBand({ range: [54, 470], count: VALUES.length, paddingInner: 0.24 });
const Y_A = scaleLinear({ domain: [0, 80], range: [350, 90] });
const Y_B = scaleLinear({ domain: [0, 100], range: [350, 90] });
const TOP_A = VALUES.map((value) => Y_A.map(value));
const TOP_B = VALUES.map((value, index) => Y_B.map(value + index * 4));
const DENSE = Array.from({ length: 1200 }, (_, index) => point(70 + (index % 60) * 8, 390 + Math.floor(index / 60) * 5));

function CardBar(ctx, { index, accent, progress }) {
  const top = TOP_A[index] + (TOP_B[index] - TOP_A[index]) * progress;
  return <View key="bar" className="absolute" style={{ left: X.band(index)[0], top, width: X.bandwidth(), height: 350 - top, backgroundColor: accent, opacity: 0.7 + index * 0.08 }} />;
}

function ChartCard(ctx, { accent, title }) {
  const p = ctx.hold.progress;
  return (
    <View key="card" className="absolute" style={{ borderStyle: "solid", width: 540, height: 440, borderRadius: 28, backgroundColor: "#0f172acc", borderWidth: 1, borderColor: accent, displacement: displacement(19, point(0.008, 0.006), 3 + p * 2, { octaves: 2, mode: "fractal" }) }}>
      <Text key="title" style={{ marginLeft: 34, marginTop: 28, fontFamily: "asset://brandFont", fontSize: 28, color: accent }}>{title}</Text>
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
      camera={{ center: point(ax + (bx - ax) * p, ay + (by - ay) * p), zoom: 1.05, rotation: 0 }}
    >
      <World key="world">
        <View key="primary-host" className="absolute" style={{ left: 90, top: 150, width: 540, height: 440 }}>
          <ChartCard key="primary" accent="#22d3ee" title="CYAN / PREPARED A→B" />
        </View>
        <View key="secondary-host" className="absolute" style={{ left: 760, top: 210, width: 540, height: 440 }}>
          <ChartCard key="secondary" accent="#a78bfa" title="VIOLET / SAME PARTS" />
        </View>
        <GeometryBatch
          key="dense-markers"
          geometry="circle"
          positions={DENSE}
          sizes={2.4}
          fills="#f8fafc"
          opacities={0.34}
          style={{ position: "absolute", left: 0, top: 0, width: 1280, height: 720, filter: "blur(0.3px)" }}
        />
      </World>
      <Screen key="hud">
        <View key="panel" className="absolute" style={{ left: 34, top: 28, width: TITLE_METRICS.width + 44, height: 64, borderRadius: 18, backgroundColor: "#020617dd" }}>
          <Text key="title" style={{ marginLeft: 22, marginTop: 13, fontFamily: "asset://brandFont", fontSize: 38, color: "#f8fafc" }}>TWO THEMES / ONE CONTRACT</Text>
        </View>
      </Screen>
    </Scene>
  );
}
