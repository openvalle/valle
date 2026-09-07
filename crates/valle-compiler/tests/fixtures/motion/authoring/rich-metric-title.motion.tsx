export const component = "rich-metric-title";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
  assets: {
    brandFont: asset({ kind: "font", required: true }),
  },
});

const reveal = defineSequence({
  label: stage({ duration: seconds(0.75) }),
  amount: stage({ after: "label", overlap: seconds(0.25), duration: seconds(1.15) }),
  detail: stage({ after: "amount", overlap: seconds(0.4), duration: seconds(1.0) }),
  chart: stage({ after: "detail", overlap: seconds(0.25), duration: seconds(1.7) }),
  settle: stage({ after: "chart", overlap: seconds(0.7), duration: seconds(2.1) }),
});

const BARS = [0.38, 0.52, 0.46, 0.68, 0.59, 0.77, 0.72, 0.91];

export default function RichMetricTitle(ctx) {
  const label = stageProgress(ctx.localFrame, ctx.fps, reveal.label);
  const amount = stageProgress(ctx.localFrame, ctx.fps, reveal.amount);
  const detail = stageProgress(ctx.localFrame, ctx.fps, reveal.detail);
  const chart = stageProgress(ctx.localFrame, ctx.fps, reveal.chart);
  const settle = yoyoProgress(ctx.localFrame, ctx.fps, reveal.settle, 2);
  const revenue = 1284500 + round(amount * 932700);
  const margin = 0.284 + settle * 0.026;

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#f2eee7", fontFamily: "asset://brandFont" }}>
      <View key="ink" className="absolute" style={{ right: -180, top: -250, width: 760, height: 760, borderRadius: 390, backgroundColor: "#172554", opacity: 0.1, filter: "blur(110px)" }} />
      <View key="orange" className="absolute" style={{ left: -190, bottom: -330, width: 720, height: 720, borderRadius: 370, backgroundColor: "#ea580c", opacity: 0.1, filter: "blur(120px)" }} />
      <Text key="edition" className="absolute" style={{ left: 68, top: 44, fontSize: 17, letterSpacing: 2, color: "#9a3412", opacity: label }}>QUARTERLY SIGNAL / Q4</Text>
      <Text key="folio" className="absolute" style={{ right: 70, top: 42, fontSize: 17, color: "#78716c" }}>VALLE / METRIC STUDY 07</Text>

      <View key="rule-top" className="absolute" style={{ left: 68, top: 90, width: 1144 * label, height: 2, backgroundColor: "#1c1917" }} />

      <Text key="headline" className="absolute" style={{ left: 66, top: 126, width: 1120, fontFamily: "asset://brandFont", fontSize: 50, color: "#292524", opacity: label, translate: point(0, (1 - label) * 22) }}>
        {"Revenue accelerated in "}<Span style={{ color: "#c2410c", fontWeight: 700 }}>every market</Span>{"."}
      </Text>

      <Text key="amount" className="absolute" style={{ left: 58, top: 220, width: 1160, fontFamily: "asset://brandFont", fontSize: 126, letterSpacing: -3, color: "#1c1917", opacity: amount, translate: point(0, (1 - amount) * 45) }}>
        <Span style={{ fontSize: 47, color: "#a8a29e", fontWeight: 700 }}>$</Span>
        <Span style={{ color: "#1c1917", fontWeight: 700 }}>{formatNumber(revenue, { decimals: 0, grouping: true })}</Span>
        <Span style={{ fontSize: 34, color: "#c2410c", letterSpacing: 0 }}> USD</Span>
      </Text>

      <Text key="delta" className="absolute" style={{ left: 71, top: 392, width: 590, fontFamily: "asset://brandFont", fontSize: 28, color: "#57534e", opacity: detail }}>
        <Span style={{ color: "#15803d", fontWeight: 700 }}>+ {formatPercent(0.176, { decimals: 1 })}</Span>{" year over year · margin "}<Span style={{ color: "#9a3412", fontWeight: 700 }}>{formatPercent(margin, { decimals: 1 })}</Span>
      </Text>

      <View key="chart" className="absolute" style={{ borderStyle: "solid", left: 69, bottom: 56, width: 1142, height: 214, borderTopWidth: 1, borderColor: "#a8a29e", opacity: chart }}>
        {BARS.map((value, i) => (
          <View key={`bar-${i}`} className="absolute" style={{ left: 24 + i * 138, bottom: 36, width: 86, height: value * 142 * staggerProgress(ctx.localFrame, ctx.fps, reveal.chart, i, seconds(0.08)), backgroundColor: i === 7 ? "#c2410c" : "#292524", opacity: 0.35 + i * 0.08 }} />
        ))}
        {BARS.map((value, i) => (
          <Text key={`month-${i}`} className="absolute" style={{ left: 24 + i * 138, bottom: 8, width: 86, textAlign: "center", fontSize: 15, color: "#78716c" }}>M{padNumber(i + 1, { width: 2 })}</Text>
        ))}
        <Text key="chart-note" className="absolute" style={{ right: 8, top: 18, fontSize: 16, color: "#9a3412" }}>RECORD CLOSE</Text>
      </View>
    </Scene>
  );
}
