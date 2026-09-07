export const component = "formula-engineering";

export default function Engineering() {
  return (
    <Scene>
      <View style={{ padding: "40px", backgroundColor: "#111827" }}>
        <Text style={{ color: "#e5e7eb", fontSize: 32 }}>传输方程（中文说明在公式外）</Text>
        <MathFormula
          latex={String.raw`\operatorname{SNR} = 10\log_{10}\frac{P_s}{P_n}`}
          displayMode="display"
          style={{ fontSize: 52, color: "#fde68a" }}
        />
        <Text style={{ color: "#9ca3af", fontSize: 24 }}>
          中文注释放在项目字体 Text 节点，不进入 MathFormula。
        </Text>
      </View>
    </Scene>
  );
}
