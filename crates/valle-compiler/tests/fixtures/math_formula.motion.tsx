export const component = "formula-card";

export default function FormulaCard() {
  return (
    <Scene>
      <MathFormula
        latex={String.raw`E = mc^2`}
        displayMode="display"
        ariaLabel="mass energy"
        style={{ fontSize: 48, color: "#111111" }}
      />
    </Scene>
  );
}
