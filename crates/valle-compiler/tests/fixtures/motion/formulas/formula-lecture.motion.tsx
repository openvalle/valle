export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "formula-lecture";

export default function Lecture() {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0b1220" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 104, top: 62, fontSize: 21, letterSpacing: 4, color: "#67e8f9" }}>FORMULA / STUDY SHEET</Text>
      <Text key="heading" className="absolute" style={{ left: 100, top: 102, fontSize: 58, color: "#f8fafc" }}>Derivation, matrix, cases, identity.</Text>
      <View key="derivation" className="absolute" style={{ borderStyle: "solid", left: 104, top: 226, width: 822, height: 340, borderRadius: 28, backgroundColor: "#141f32", borderWidth: 1, borderColor: "#34445d" }}>
        <Text key="derivation-label" className="absolute" style={{ left: 48, top: 32, fontSize: 20, color: "#94a3b8" }}>01 / DERIVATION</Text>
        <MathFormula
          latex={String.raw`\begin{aligned}
            E &= mc^2 \\
            F &= \frac{d}{dt}(mv)
          \end{aligned}`}
          displayMode="display"
          ariaLabel="energy and force"
          style={{ position: "absolute", left: 50, top: 88, fontSize: 56, color: "#f8fafc" }}
        />
      </View>
      <View key="matrix" className="absolute" style={{ borderStyle: "solid", left: 994, top: 226, width: 822, height: 340, borderRadius: 28, backgroundColor: "#141f32", borderWidth: 1, borderColor: "#34445d" }}>
        <Text key="matrix-label" className="absolute" style={{ left: 48, top: 32, fontSize: 20, color: "#94a3b8" }}>02 / MATRIX</Text>
        <MathFormula
          latex={String.raw`\begin{pmatrix} a & b \\ c & d \end{pmatrix}\begin{pmatrix} x \\ y \end{pmatrix}`}
          displayMode="display"
          style={{ position: "absolute", left: 50, top: 112, fontSize: 50, color: "#67e8f9" }}
        />
      </View>
      <View key="cases" className="absolute" style={{ borderStyle: "solid", left: 104, top: 610, width: 822, height: 340, borderRadius: 28, backgroundColor: "#141f32", borderWidth: 1, borderColor: "#34445d" }}>
        <Text key="cases-label" className="absolute" style={{ left: 48, top: 32, fontSize: 20, color: "#94a3b8" }}>03 / PIECEWISE</Text>
        <MathFormula
          latex={String.raw`f(x)=\begin{cases} x^{2} & x\ge 0 \\ -x & x<0 \end{cases}`}
          displayMode="display"
          ariaLabel="piecewise square"
          style={{ position: "absolute", left: 50, top: 96, fontSize: 49, color: "#f8fafc" }}
        />
      </View>
      <View key="identity" className="absolute" style={{ borderStyle: "solid", left: 994, top: 610, width: 822, height: 340, borderRadius: 28, backgroundColor: "#141f32", borderWidth: 1, borderColor: "#34445d" }}>
        <Text key="identity-label" className="absolute" style={{ left: 48, top: 32, fontSize: 20, color: "#94a3b8" }}>04 / IDENTITY</Text>
        <MathFormula
          latex={String.raw`\begin{equation*}
            \color{magenta}{c^{2}=a^{2}+b^{2}}\tag{P}
          \end{equation*}`}
          displayMode="display"
          ariaLabel="pythagoras tagged"
          style={{ position: "absolute", left: 50, top: 112, fontSize: 49, color: "#f8fafc" }}
        />
      </View>
    </Scene>
  );
}
