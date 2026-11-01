export const composition = { width: 360, height: 210, fps: 1, duration: 1 };

// Inline SVG shapes, filled: a rounded rect, a circle, an ellipse and one with a single radius;
// a pentagram by nonzero and by evenodd; an open polyline, which filling closes; a path of
// curves and arcs; and a line, which fills nothing. A group's fill inherits, and its opacity
// takes what it draws as one layer; transforms nest. currentColor is each shape's own color;
// fill-opacity scales the fill; percentages measure the viewport; in a hidden group a visible
// shape shows; style beats a presentation attribute; display: none draws nothing.
export default function Scene() {
  const star = "50,5 79,95 3,40 97,40 21,95";
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8, background: "#eceff1" }}>
      <svg width={100} height={60} viewBox="0 0 100 60">
        <rect x={4} y={4} width={36} height={52} rx={10} fill="#3949ab" />
        <circle cx={66} cy={18} r={14} fill="#e53935" />
        <ellipse cx={66} cy={46} rx={28} ry={9} fill="#43a047" fillOpacity={0.6} />
        <ellipse cx={92} cy={18} rx={6} fill="#fb8c00" />
      </svg>
      <svg width={60} height={60} viewBox="0 0 100 100" fill="#8e24aa">
        <polygon points={star} />
      </svg>
      <svg width={60} height={60} viewBox="0 0 100 100" fill="#8e24aa" fillRule="evenodd">
        <polygon points={star} />
      </svg>
      <svg width={100} height={60}>
        <polyline points="4,56 30,4 56,56 70,20" fill="#fdd835" />
        <line x1={4} y1={4} x2={96} y2={56} fill="#e53935" />
        <path
          d="M60 50 Q 70 30 80 50 T 96 40 A 10 14 30 0 1 80 58 C 70 62 64 58 60 50 Z"
          fill="#00897b"
        />
      </svg>
      <svg width={100} height={70} viewBox="0 0 50 35">
        <g fill="#5c6bc0" opacity={0.5} transform="translate(4 4)">
          <rect width={20} height={14} />
          <rect x={10} y={8} width={20} height={14} />
        </g>
        <g transform="translate(36 6) rotate(30)">
          <rect width={10} height={10} fill="#e53935" transform="scale(1.2, 0.6) skewX(20)" />
        </g>
        <rect x={34} y={20} width={10} height={10} fill="#43a047" transform="rotate(45 39 25)" />
        <path d="M2 30 h10 v4 h-10 z" fill="#fb8c00" transform="matrix(1 0 0.5 1 -12 0)" />
      </svg>
      <svg width={80} height={60} style={{ color: "#1e88e5" }}>
        <g fill="currentColor">
          <circle cx="25%" cy="50%" r="10%" />
          <circle cx="75%" cy="50%" r="10%" color="#d81b60" />
        </g>
        <rect x="10%" y="5%" width="80%" height="10%" fill="#7cb342" fillOpacity="50%" />
      </svg>
      <svg width={80} height={60} viewBox="0 0 40 30">
        <g visibility="hidden" fill="#e53935">
          <rect x={2} y={2} width={16} height={12} />
          <rect x={22} y={2} width={16} height={12} visibility="visible" />
        </g>
        <rect x={2} y={16} width={16} height={12} fill="#e53935" style={{ fill: "#1e88e5" }} />
        <rect x={22} y={16} width={16} height={12} fill="#e53935" style={{ display: "none" }} />
      </svg>
    </View>
  );
}
