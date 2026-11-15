export const composition = { width: 480, height: 300, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Latin: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 480, height: 300, backgroundColor: "white" }}>
      <svg width="480" height="300" fontFamily="Latin, CJK" fontSize="28" fill="#17365d">
        <path
          d="M50 10V280 M120 10V280 M190 10V280 M260 10V280 M330 10V280 M400 10V280"
          stroke="#ddd"
        />
        <text x="50" y="20" writingMode="vertical-rl">
          山水ABC
        </text>
        <text x="120" y="20" writingMode="vertical-lr">
          山水ABC
        </text>
        <text x="190" y="20" writingMode="vertical-rl" style={{ textOrientation: "upright" }}>
          ABC山水
        </text>
        <text x="260" y="20" writingMode="vertical-rl" style={{ textOrientation: "sideways" }}>
          ABC山水
        </text>
        <text x="330" y="150" writingMode="vertical-rl" textAnchor="middle">
          山水ABC
        </text>
        <text
          x="400"
          y="20 70 120"
          writingMode="vertical-rl"
          style={{ textOrientation: "upright" }}
        >
          ABC
        </text>
      </svg>
    </View>
  );
}
