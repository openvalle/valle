export const composition = { width: 400, height: 180, fps: 1, duration: 1 };

// Logos and badges where everything meets: a gradient shield with an inner stroke clipped to it
// and a highlight masked away; an emblem of turned copies, some translucent; a ring of dashes
// around a gradient disc; a ribbon with its folds, gradients and an even-odd star; and a card with
// a mask of a gradient, a clip path and markers on one group.
export default function Scene() {
  const size = { width: 120, height: 120, viewBox: "0 0 120 120" };
  const shield = "M60 8 L104 22 V60 C104 86 84 104 60 112 C36 104 16 86 16 60 V22 Z";
  return (
    <View style={{ display: "flex", gap: 8, padding: 8, background: "#eceff1" }}>
      <svg {...size}>
        <defs>
          <linearGradient id="b1" x1={0} y1={0} x2={1} y2={1}>
            <stop offset={0} stopColor="#42a5f5" />
            <stop offset={1} stopColor="#1a237e" />
          </linearGradient>
          <linearGradient id="b2" x1={0} y1={0} x2={0} y2={1}>
            <stop offset={0} stopColor="#fff" />
            <stop offset={1} stopColor="#000" />
          </linearGradient>
          <clipPath id="b3">
            <path d={shield} />
          </clipPath>
          <mask id="b4">
            <rect width={120} height={60} fill="url(#b2)" />
          </mask>
        </defs>
        <path d={shield} fill="url(#b1)" />
        <path d={shield} fill="none" stroke="#fdd835" strokeWidth={8} clipPath="url(#b3)" />
        <ellipse cx={60} cy={30} rx={50} ry={26} fill="#fff" opacity={0.5} mask="url(#b4)" />
        <path
          d="M40 60 L54 76 L82 42"
          fill="none"
          stroke="#fff"
          strokeWidth={8}
          strokeLinecap="round"
        />
      </svg>
      <svg {...size}>
        <defs>
          <ellipse id="b5" cx={60} cy={30} rx={12} ry={26} />
        </defs>
        <g fill="#e53935" stroke="#7f0000" strokeWidth={2}>
          {[0, 45, 90, 135, 180, 225, 270, 315].map((a, i) => (
            <use
              key={a}
              href="#b5"
              transform={`rotate(${a} 60 60)`}
              fill={i % 2 ? "#fb8c00" : undefined}
              opacity={i % 2 ? 0.7 : 1}
            />
          ))}
        </g>
        <circle cx={60} cy={60} r={10} fill="#fdd835" />
      </svg>
      <svg width={120} height={120} viewBox="0 0 60 60">
        <defs>
          <radialGradient id="b6" cx={0.35} cy={0.35} r={0.75}>
            <stop offset={0} stopColor="#fff59d" />
            <stop offset={0.6} stopColor="#fb8c00" />
            <stop offset={1} stopColor="#b71c1c" />
          </radialGradient>
        </defs>
        <circle cx={30} cy={30} r={20} fill="url(#b6)" />
        <circle
          cx={30}
          cy={30}
          r={26}
          fill="none"
          stroke="#37474f"
          strokeWidth={3}
          strokeDasharray="2 3.2"
          strokeLinecap="round"
          transform="rotate(10 30 30)"
        />
        <polygon
          points="30,16 34,40 14,25 46,25 26,40"
          fill="#fff"
          fillRule="evenodd"
          fillOpacity={0.85}
        />
      </svg>
    </View>
  );
}
