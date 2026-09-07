// Formula wall: many locked KaTeX faces on one board.
//
//   cargo run -p valle-cli --bin valle -- motion check \
//     crates/valle-compiler/tests/fixtures/motion/formulas/formula-wall.motion.tsx \
//     --canvas-width 1920 --canvas-height 1080

export const component = "formula-wall";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const CARDS = [
  {
    title: "MASS-ENERGY",
    latex: String.raw`E = mc^{2}`,
    color: "#f8fafc",
  },
  {
    title: "QUADRATIC",
    latex: String.raw`x = \frac{-b \pm \sqrt{b^{2}-4ac}}{2a}`,
    color: "#fde68a",
  },
  {
    title: "EULER",
    latex: String.raw`e^{i\pi} + 1 = 0`,
    color: "#67e8f9",
  },
  {
    title: "GAUSSIAN",
    latex: String.raw`\frac{1}{\sqrt{2\pi\sigma^{2}}}\,e^{-\frac{(x-\mu)^{2}}{2\sigma^{2}}}`,
    color: "#f8fafc",
  },
  {
    title: "MATRIX",
    latex: String.raw`\begin{pmatrix} a & b \\ c & d \end{pmatrix}\begin{pmatrix} x \\ y \end{pmatrix}`,
    color: "#67e8f9",
  },
  {
    title: "CASES",
    latex: String.raw`f(x)=\begin{cases} x^{2} & x\ge 0 \\ -x & x<0 \end{cases}`,
    color: "#f8fafc",
  },
  {
    title: "ALIGNED",
    latex: String.raw`\begin{aligned} \nabla\cdot\mathbf{E} &= \rho/\varepsilon_{0} \\ \nabla\times\mathbf{E} &= -\partial_{t}\mathbf{B} \end{aligned}`,
    color: "#fde68a",
  },
  {
    title: "GAUSS INTEGRAL",
    latex: String.raw`\int_{-\infty}^{\infty} e^{-x^{2}}\,dx = \sqrt{\pi}`,
    color: "#67e8f9",
  },
  {
    title: "BASEL",
    latex: String.raw`\sum_{n=1}^{\infty}\frac{1}{n^{2}} = \frac{\pi^{2}}{6}`,
    color: "#f8fafc",
  },
  {
    title: "SCHRODINGER",
    latex: String.raw`i\hbar\partial_{t}\Psi = \hat{H}\Psi`,
    color: "#fde68a",
  },
  {
    title: "FOURIER",
    latex: String.raw`\hat{f}(\xi)=\int_{-\infty}^{\infty}f(x)e^{-2\pi ix\xi}\,dx`,
    color: "#67e8f9",
  },
  {
    title: "TAG + COLOR",
    latex: String.raw`\begin{equation*}\color{magenta}{c^{2}=a^{2}+b^{2}}\tag{P}\end{equation*}`,
    color: "#f8fafc",
  },
];

export default function FormulaWall(ctx) {
  const t = ctx.hold.progress;
  const header = interpolate(t, [0, 0.08], [0, 1], { easing: "easeOut" });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#05080f" }}>
      <View
        key="glow"
        className="absolute"
        style={{
          left: 420,
          top: -180,
          width: 1080,
          height: 420,
          borderRadius: 280,
          backgroundColor: "#0ea5e9",
          opacity: 0.12,
          filter: "blur(90px)",
        }}
      />
      <View
        key="header"
        className="absolute flex items-center"
        style={{ left: 40, top: 28, width: 1840, height: 64, opacity: header }}
      >
        <View
          key="dot"
          style={{
            width: 12,
            height: 12,
            borderRadius: 6,
            backgroundColor: "#22d3ee",
          }}
        />
        <Text key="brand" style={{ marginLeft: 14, fontSize: 26, color: "#e2e8f0" }}>
          VALLE / MATHFORMULA
        </Text>
        <Text key="note" style={{ marginLeft: 36, fontSize: 18, color: "#64748b" }}>
          十二道式 · 中文只在标题里
        </Text>
      </View>
      {CARDS.map((card, i) => (
        <View
          key={card.title}
          className="absolute"
          style={{
            borderStyle: "solid", left: 36 + (i % 3) * 624,
            top: 112 + ((i - (i % 3)) / 3) * 236,
            width: 604,
            height: 220,
            borderRadius: 18,
            backgroundColor: "#0b1220ee",
            borderWidth: 1,
            borderColor: "#1e293b",
            opacity: interpolate(t - i * 0.042, [0, 0.13], [0, 1], {
              easing: "cubic-bezier(0.22, 1, 0.36, 1)",
            }),
            translate: point(
              0,
              (1 -
                interpolate(t - i * 0.042, [0, 0.13], [0, 1], {
                  easing: "cubic-bezier(0.22, 1, 0.36, 1)",
                })) *
                26,
            ),
            padding: 16,
          }}
        >
          <Text key={`label-${card.title}`} style={{ fontSize: 14, color: "#64748b" }}>
            {card.title}
          </Text>
          <MathFormula
            key={`tex-${card.title}`}
            latex={card.latex}
            displayMode="display"
            style={{ fontSize: 28, color: card.color }}
          />
        </View>
      ))}
    </Scene>
  );
}
