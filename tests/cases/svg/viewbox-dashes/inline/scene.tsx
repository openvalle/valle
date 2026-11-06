export const composition = { width: 340, height: 260, fps: 1, duration: 1 };

export default function ViewboxDashes() {
  const drawing = (
    <>
      <rect width={40} height={20} fill="#176bba" />
      <rect x={8} y={4} width={24} height={12} fill="#ffb74d" />
    </>
  );
  const variants = [
    { key: "meet-center", aspect: "xMidYMid meet" },
    { key: "slice", aspect: "xMinYMin slice" },
    { key: "stretch", aspect: "none" },
    { key: "meet-end", aspect: "xMaxYMax meet" },
  ];
  return (
    <View style={{ width: 340, height: 260, backgroundColor: "white" }}>
      <svg
        key="drawing"
        viewBox="0 0 120 60"
        style={{ position: "absolute", left: 20, top: 20, width: 240, height: 120 }}
      >
        <rect width={120} height={60} fill="#eef2f7" />
        <path
          d="M10 16H110"
          pathLength={10}
          stroke="#176bba"
          strokeWidth={4}
          strokeDasharray="1 1"
        />
        <g transform="translate(10 32)">
          <path d="M0 0H100V16H0Z M10 4H90V12H10Z" fill="#16a085" fillRule="evenodd" />
        </g>
      </svg>
      {variants.map((variant, i) => (
        <svg
          key={variant.key}
          viewBox="0 0 40 20"
          preserveAspectRatio={variant.aspect}
          style={{
            position: "absolute",
            left: 20 + i * 80,
            top: 180,
            width: 60,
            height: 60,
            backgroundColor: "#eef2f7",
            overflow: "hidden",
          }}
        >
          {drawing}
        </svg>
      ))}
    </View>
  );
}
