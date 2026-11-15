export const composition = { width: 420, height: 320, fps: 1, duration: 1 };
export const controls = {
  resources: { Latin: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 420, height: 320, backgroundColor: "white" }}>
      <svg width="420" height="320" fontFamily="Latin" fontSize="44" fill="#17365d">
        <defs>
          <path id="arc" d="M20 150 Q210 -20 400 150" />
          <path id="lower" d="M20 280 Q210 110 400 280" />
        </defs>
        <use href="#arc" fill="none" stroke="#ddd" />
        <use href="#lower" fill="none" stroke="#ddd" />
        <text textAnchor="middle">
          <textPath href="#arc" startOffset="50%" method="align">
            WAVES
          </textPath>
        </text>
        <text textAnchor="middle">
          <textPath href="#lower" startOffset="50%" method="stretch">
            WAVES
          </textPath>
        </text>
      </svg>
    </View>
  );
}
