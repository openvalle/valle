export const component = "formula-lecture";

export default function Lecture() {
  return (
    <Scene>
      <View style={{ padding: "48px", backgroundColor: "#0b1220" }}>
        <Text style={{ color: "#94a3b8", fontSize: 28 }}>Derivation</Text>
        <MathFormula
          latex={String.raw`\begin{aligned}
            E &= mc^2 \\
            F &= \frac{d}{dt}(mv)
          \end{aligned}`}
          displayMode="display"
          ariaLabel="energy and force"
          style={{ fontSize: 56, color: "#f8fafc" }}
        />
        <MathFormula
          latex={String.raw`\begin{pmatrix} a & b \\ c & d \end{pmatrix}\begin{pmatrix} x \\ y \end{pmatrix}`}
          displayMode="display"
          style={{ fontSize: 48, color: "#67e8f9" }}
        />
        <MathFormula
          latex={String.raw`f(x)=\begin{cases} x^{2} & x\ge 0 \\ -x & x<0 \end{cases}`}
          displayMode="display"
          ariaLabel="piecewise square"
          style={{ fontSize: 48, color: "#f8fafc" }}
        />
        <MathFormula
          latex={String.raw`\begin{equation*}
            \color{magenta}{c^{2}=a^{2}+b^{2}}\tag{P}
          \end{equation*}`}
          displayMode="display"
          ariaLabel="pythagoras tagged"
          style={{ fontSize: 48 }}
        />
      </View>
    </Scene>
  );
}
