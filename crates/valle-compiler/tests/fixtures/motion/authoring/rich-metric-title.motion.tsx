export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

export const controls = {
  assets: {
    brandFont: asset({ kind: "font", required: true }),
  },
};

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
      <View key="ink" className="absolute" style={{ right: -270, top: -375, width: 1140, height: 1140, borderRadius: 585, backgroundColor: "#172554", opacity: 0.1, filter: "blur(165px)" }} />
      <View key="orange" className="absolute" style={{ left: -285, bottom: -495, width: 1080, height: 1080, borderRadius: 555, backgroundColor: "#ea580c", opacity: 0.1, filter: "blur(180px)" }} />
      <Text key="edition" className="absolute" style={{ left: 102, top: 66, fontSize: 25.5, letterSpacing: 3, color: "#9a3412", opacity: label }}>QUARTERLY SIGNAL / Q4</Text>
      <Text key="folio" className="absolute" style={{ right: 105, top: 63, fontSize: 25.5, color: "#78716c" }}>VALLE / METRIC STUDY 07</Text>

      <View key="rule-top" className="absolute" style={{ left: 102, top: 135, width: 1716 * label, height: 3, backgroundColor: "#1c1917" }} />

      <Text key="headline" className="absolute" style={{ left: 99, top: 189, width: 1680, fontFamily: "asset://brandFont", fontSize: 75, color: "#292524", opacity: label, translate: point(0, (1 - label) * 33) }}>
        {"Revenue accelerated in "}<Span style={{ color: "#c2410c", fontWeight: 700 }}>every market</Span>{"."}
      </Text>

      <Text key="amount" className="absolute" style={{ left: 87, top: 330, width: 1740, fontFamily: "asset://brandFont", fontSize: 189, letterSpacing: -4.5, color: "#1c1917", opacity: amount, translate: point(0, (1 - amount) * 67.5) }}>
        <Span style={{ fontSize: 70.5, color: "#a8a29e", fontWeight: 700 }}>$</Span>
        <Span style={{ color: "#1c1917", fontWeight: 700 }}>{formatNumber(revenue, { decimals: 0, grouping: true })}</Span>
        <Span style={{ fontSize: 51, color: "#c2410c", letterSpacing: 0 }}> USD</Span>
      </Text>

      <Text key="delta" className="absolute" style={{ left: 106.5, top: 588, width: 885, fontFamily: "asset://brandFont", fontSize: 42, color: "#57534e", opacity: detail }}>
        <Span style={{ color: "#15803d", fontWeight: 700 }}>+ {formatPercent(0.176, { decimals: 1 })}</Span>{" year over year · margin "}<Span style={{ color: "#9a3412", fontWeight: 700 }}>{formatPercent(margin, { decimals: 1 })}</Span>
      </Text>

      <View key="chart" className="absolute" style={{ borderStyle: "solid", left: 103.5, bottom: 84, width: 1713, height: 321, borderTopWidth: 1.5, borderColor: "#a8a29e", opacity: chart }}>
        {BARS.map((value, i) => (
          <View key={`bar-${i}`} className="absolute" style={{ left: 36 + i * 207, bottom: 54, width: 129, height: value * 213 * staggerProgress(ctx.localFrame, ctx.fps, reveal.chart, i, seconds(0.08)), backgroundColor: i === 7 ? "#c2410c" : "#292524", opacity: 0.35 + i * 0.08 }} />
        ))}
        {BARS.map((value, i) => (
          <Text key={`month-${i}`} className="absolute" style={{ left: 36 + i * 207, bottom: 12, width: 129, textAlign: "center", fontSize: 22.5, color: "#78716c" }}>M{padNumber(i + 1, { width: 2 })}</Text>
        ))}
        <Text key="chart-note" className="absolute" style={{ right: 12, top: 27, fontSize: 24, color: "#9a3412" }}>RECORD CLOSE</Text>
      </View>
    </Scene>
  );
}
