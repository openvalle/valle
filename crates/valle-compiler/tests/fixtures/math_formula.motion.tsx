export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

export default function FormulaCard() {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#080f1e" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 144, top: 104, fontSize: 24, letterSpacing: 5, color: "#67e8f9" }}>MOTION / MATH FORMULA</Text>
      <Text key="title" className="absolute" style={{ left: 140, top: 164, fontSize: 66, color: "#f8fafc" }}>A compact idea, given room to breathe.</Text>
      <View key="formula-card" className="absolute" style={{ borderStyle: "solid", left: 144, top: 306, width: 1632, height: 604, borderRadius: 36, backgroundColor: "#111d31", borderWidth: 2, borderColor: "#26445a" }}>
        <Text key="formula-label" className="absolute" style={{ left: 74, top: 62, fontSize: 24, letterSpacing: 3, color: "#94a3b8" }}>MASS–ENERGY EQUIVALENCE</Text>
        <MathFormula
          latex={String.raw`E = mc^2`}
          displayMode="display"
          ariaLabel="mass energy"
          style={{ position: "absolute", left: 130, top: 198, fontSize: 128, color: "#f8fafc" }}
        />
        <View key="accent-rule" className="absolute" style={{ left: 76, bottom: 82, width: 1480, height: 2, backgroundColor: "#26445a" }} />
        <Text key="formula-note" className="absolute" style={{ left: 76, bottom: 34, fontSize: 23, color: "#94a3b8" }}>The formula stays a single accessible visual node.</Text>
      </View>
    </Scene>
  );
}
