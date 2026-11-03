export const composition = { width: 360, height: 240, fps: 1, duration: 1 };

// What nesting does to SVG: transforms of every kind stacked; opacities of groups inside groups
// over a stripe; a gradient in user units and one in the box's units under a turn; a clip path and
// a mask inside scaled groups; strokes scaled unevenly; paint inherited through groups and put
// back by a child; fill, stroke and group opacity on one shape; and style over attributes down
// the tree.
export default function Scene() {
  const box = { width: 104, height: 70, viewBox: "0 0 104 70" };
  const backdrop = <rect y={30} width={104} height={10} fill="#263238" />;
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8, background: "#eceff1" }}>
      <svg {...box}>
        <g transform="translate(52 35)">
          <g transform="rotate(30) scale(1.2 0.8)">
            <g transform="skewX(20)">
              <rect x={-24} y={-14} width={48} height={28} fill="#1e88e5" />
              <g transform="matrix(0.5 0 0 0.5 6 2) skewY(-15)">
                <rect x={-20} y={-14} width={40} height={28} fill="#fdd835" />
              </g>
            </g>
          </g>
        </g>
      </svg>
      <svg {...box}>
        {backdrop}
        <g opacity={0.8}>
          <rect x={8} y={8} width={40} height={54} fill="#e53935" />
          <g opacity={0.5}>
            <rect x={28} y={16} width={40} height={38} fill="#43a047" />
            <circle cx={70} cy={35} r={22} fill="#1e88e5" opacity={0.5} />
          </g>
        </g>
      </svg>
      <svg {...box}>
        <defs>
          <linearGradient id="n1" gradientUnits="userSpaceOnUse" x1={0} y1={0} x2={104} y2={0}>
            <stop offset={0} stopColor="#e53935" />
            <stop offset={1} stopColor="#1e88e5" />
          </linearGradient>
          <linearGradient id="n2">
            <stop offset={0} stopColor="#fdd835" />
            <stop offset={1} stopColor="#8e24aa" />
          </linearGradient>
        </defs>
        <g transform="rotate(20 52 35)">
          <rect x={10} y={8} width={84} height={20} fill="url(#n1)" />
          <rect x={10} y={40} width={84} height={20} fill="url(#n2)" />
        </g>
      </svg>
      <svg {...box}>
        <clipPath id="n3">
          <circle cx={26} cy={17} r={16} />
        </clipPath>
        <g transform="scale(2)">
          <g clipPath="url(#n3)" transform="translate(0 1)">
            <rect width={52} height={34} fill="#00897b" />
            <rect x={20} width={8} height={34} fill="#fff" />
          </g>
        </g>
      </svg>
      <svg {...box}>
        <mask id="n4">
          <rect width={52} height={35} fill="#808080" />
          <circle cx={26} cy={17} r={12} fill="#fff" />
        </mask>
        <g transform="translate(4 4) scale(1.8)">
          <rect width={52} height={32} fill="#fb8c00" mask="url(#n4)" />
        </g>
      </svg>
      <svg {...box}>
        <g transform="scale(3 1)" fill="none" stroke="#3949ab" strokeWidth={3}>
          <circle cx={12} cy={35} r={8} />
          <path d="M26 12 H10 V24" strokeLinejoin="round" />
        </g>
      </svg>
      <svg {...box}>
        <g fill="#1e88e5" stroke="#e53935" strokeWidth={3}>
          <g fill="#43a047">
            <rect x={6} y={8} width={26} height={54} />
            <g stroke="none">
              <rect x={38} y={8} width={26} height={54} />
              <rect x={70} y={8} width={26} height={54} fill="#fdd835" stroke="#263238" />
            </g>
          </g>
        </g>
      </svg>
      <svg {...box}>
        {backdrop}
        <circle
          cx={52}
          cy={35}
          r={26}
          fill="#1e88e5"
          fillOpacity={0.6}
          stroke="#e53935"
          strokeOpacity={0.6}
          strokeWidth={10}
          opacity={0.7}
        />
      </svg>
      <svg {...box}>
        <g style={{ fill: "#8e24aa" }} fill="#fdd835">
          <rect x={6} y={8} width={26} height={54} />
          <rect x={38} y={8} width={26} height={54} fill="#e53935" />
          <rect x={70} y={8} width={26} height={54} style={{ fill: "#43a047" }} fill="#e53935" />
        </g>
      </svg>
    </View>
  );
}
