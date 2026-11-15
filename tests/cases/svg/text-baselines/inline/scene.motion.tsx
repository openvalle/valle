export const composition = { width: 480, height: 330, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 480, height: 330, background: "white" }}>
      <svg width="480" height="330" fontFamily="sans-serif" fontSize="28" fill="#172554">
        <path d="M12 55H465M12 120H465M12 180H465" stroke="#cbd5e1" />
        <text x="16" y="55">
          x
          <tspan fontSize="18" baselineShift="super">
            2
          </tspan>
          <tspan dx="20">H</tspan>
          <tspan fontSize="18" baselineShift="sub">
            2
          </tspan>
          O
          <tspan dx="20" baselineShift={8 + seconds * 4}>
            up<tspan baselineShift="25%">+</tspan>
          </tspan>
          end
        </text>
        <text x="16" y="120" fontSize="24">
          A
          <tspan alignmentBaseline="middle" fontSize="18">
            middle
          </tspan>
          <tspan dx="12" alignmentBaseline="text-before-edge" fontSize="18">
            top
          </tspan>
          <tspan dx="12" alignmentBaseline="text-after-edge" fontSize="18">
            bottom
          </tspan>
        </text>
        <text x="16" y="180" textDecoration="underline" fill="#2563eb">
          blue<tspan fill="#dc2626"> red</tspan>
          <tspan textDecoration="line-through" dx="10">
            strike
          </tspan>
        </text>
        <text
          x="16"
          y="235"
          textDecoration="underline overline"
          fill="#15803d"
          stroke="#14532d"
          strokeWidth="0.4"
        >
          Lines
        </text>
        <text
          x="145"
          y="235"
          style={{ textDecoration: "underline double", textDecorationThickness: 2 }}
        >
          double
        </text>
        <text
          x="275"
          y="235"
          style={{ textDecoration: "underline wavy", textDecorationThickness: 2 }}
        >
          wave
        </text>
        <defs>
          <path id="curve" d="M16 300Q235 250 450 300" />
          <text id="copy" textDecoration="underline">
            use
          </text>
          <clipPath id="letters">
            <text x="390" y="240" fontSize="32" textDecoration="underline">
              VA
            </text>
          </clipPath>
        </defs>
        <text fontSize="24" textDecoration="underline" fill="#7e22ce">
          <textPath href="#curve">Text following a curved line</textPath>
        </text>
        <use href="#copy" x="385" y="180" fontSize="24" fill="#be123c" />
        <rect x="380" y="205" width="80" height="45" fill="#ea580c" clipPath="url(#letters)" />
      </svg>
    </View>
  );
}
