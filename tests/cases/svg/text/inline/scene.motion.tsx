export const composition = { width: 360, height: 260, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 360, height: 260, backgroundColor: "white" }}>
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="360"
        height="260"
        viewBox="0 0 360 260"
        fontFamily="sans-serif"
        fontSize="24"
        fill="#17324d"
      >
        <defs>
          <path id="arc" d="M25 210Q180 70 335 210" />
          <linearGradient id="ink">
            <stop stopColor="#356fe0" />
            <stop offset="1" stopColor="#d34c88" />
          </linearGradient>
        </defs>
        <path d="M180 10V100M10 125H350M25 210Q180 70 335 210" stroke="#dce4ed" fill="none" />
        <text x="180" y="32" textAnchor="middle">
          Valle{" "}
          <tspan fill="#d34c88" fontWeight="700">
            SVG
          </tspan>
        </text>
        <text x="180" y="68" textAnchor="end" fill="url(#ink)" stroke="#17324d" strokeWidth="0.5">
          Vector <tspan>text</tspan>
        </text>
        <text x="20 55 90" y="105" rotate="-10 0 10" fontSize="27">
          ABC
          <tspan dx="15" dy="8" fill="#16816b">
            def
          </tspan>
        </text>
        <text x="190" y="125" dominantBaseline="middle" fontSize="20">
          Middle baseline
        </text>
        <text x="20" y="244" textLength="140" lengthAdjust="spacingAndGlyphs" fontSize="20">
          Scaled glyphs
        </text>
        <text x="190" y="244" textLength="145" lengthAdjust="spacing" fontSize="20">
          Spaced text
        </text>
        <text fill="#356fe0" fontSize="22" textAnchor="middle">
          <textPath href="#arc" startOffset={`${50 + seconds * 8}%`}>
            Text follows a curve
          </textPath>
        </text>
      </svg>
    </View>
  );
}
