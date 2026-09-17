export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "formula-engineering";

export default function Engineering() {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0b1220" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 144, top: 104, fontSize: 24, letterSpacing: 5, color: "#fbbf24" }}>ENGINEERING / SIGNAL</Text>
      <Text key="title" className="absolute" style={{ left: 140, top: 166, fontSize: 64, color: "#f8fafc" }}>传输方程</Text>
      <Text key="subtitle" className="absolute" style={{ left: 144, top: 254, fontSize: 28, color: "#94a3b8" }}>中文说明在公式外，数学表达保持独立。</Text>
      <View key="equation-card" className="absolute" style={{ borderStyle: "solid", left: 144, top: 364, width: 1632, height: 520, borderRadius: 34, backgroundColor: "#172033", borderWidth: 2, borderColor: "#3f3e46" }}>
        <Text key="label" className="absolute" style={{ left: 68, top: 54, fontSize: 23, letterSpacing: 3, color: "#fbbf24" }}>SIGNAL-TO-NOISE RATIO</Text>
        <MathFormula
          latex={String.raw`\operatorname{SNR} = 10\log_{10}\frac{P_s}{P_n}`}
          displayMode="display"
          style={{ position: "absolute", left: 82, top: 166, fontSize: 90, color: "#fde68a" }}
        />
        <Text key="note" className="absolute" style={{ left: 68, bottom: 54, fontSize: 26, color: "#aab8cc" }}>
          中文注释放在项目字体 Text 节点，不进入 MathFormula。
        </Text>
      </View>
    </Scene>
  );
}
