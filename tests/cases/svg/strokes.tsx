export const composition = { width: 360, height: 310, fps: 1, duration: 1 };

// Strokes: butt, round and square caps; miter, round and bevel joins, a miter past its limit;
// dashes on a circle by its path length, on a rect from its corner and on a rounded one; dots of
// no length, round along a line and square along a diagonal, and subpaths of no length as dots;
// a half-transparent stroke over its fill; a group scaled more across than down, its stroke
// too; a viewBox's scale; curves joined round; and currentColor.
export default function Scene() {
  const line = { fill: "none", stroke: "#3949ab", strokeWidth: 6 } as const;
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8, background: "#eceff1" }}>
      <svg width={80} height={70}>
        <path d="M14 12 H66" {...line} strokeLinecap="butt" />
        <path d="M14 34 H66" {...line} strokeLinecap="round" />
        <path d="M14 56 H66" {...line} strokeLinecap="square" />
      </svg>
      <svg width={110} height={70}>
        <g {...line} strokeWidth={8}>
          <polyline points="8,30 20,8 32,30" strokeLinejoin="miter" />
          <polyline points="42,30 54,8 66,30" strokeLinejoin="round" />
          <polyline points="76,30 88,8 100,30" strokeLinejoin="bevel" />
          <polyline points="8,62 50,50 8,40" strokeWidth={4} />
          <polyline points="60,62 102,50 60,40" strokeWidth={4} strokeMiterlimit={12} />
        </g>
      </svg>
      <svg width={150} height={70}>
        <circle
          cx={34}
          cy={35}
          r={26}
          fill="none"
          stroke="#e53935"
          strokeWidth={5}
          pathLength={100}
          strokeDasharray="25 10"
          strokeDashoffset={5}
        />
        <rect
          x={70}
          y={8}
          width={30}
          height={54}
          fill="none"
          stroke="#43a047"
          strokeWidth={3}
          strokeDasharray="8 4"
        />
        <rect
          x={110}
          y={8}
          width={32}
          height={54}
          rx={10}
          fill="none"
          stroke="#8e24aa"
          strokeWidth={3}
          strokeDasharray="10 5 2 5"
        />
      </svg>
      <svg width={110} height={70}>
        <path
          d="M8 12 H102"
          stroke="#fb8c00"
          strokeWidth={6}
          strokeLinecap="round"
          strokeDasharray="0 12"
        />
        <path
          d="M8 26 L60 62"
          stroke="#00897b"
          strokeWidth={6}
          strokeLinecap="square"
          strokeDasharray="0 14"
        />
        <path d="M76 34 Z M92 34 Z" stroke="#3949ab" strokeWidth={10} strokeLinecap="round" />
        <path d="M76 56 Z M92 56 Z" stroke="#3949ab" strokeWidth={10} strokeLinecap="square" />
      </svg>
      <svg width={80} height={70}>
        <rect
          x={10}
          y={10}
          width={60}
          height={50}
          fill="#fdd835"
          stroke="#1e88e5"
          strokeWidth={10}
          strokeOpacity={0.5}
        />
      </svg>
      <svg width={110} height={70}>
        <g transform="translate(55 35) scale(2 1)">
          <circle r={20} fill="none" stroke="#d81b60" strokeWidth={4} />
        </g>
      </svg>
      <svg width={60} height={60} viewBox="0 0 10 10">
        <path
          d="M2 8 L5 2 L8 8"
          fill="none"
          stroke="#5c6bc0"
          strokeWidth={1}
          strokeLinejoin="round"
        />
      </svg>
      <svg width={90} height={60} style={{ color: "#c62828" }}>
        <path
          d="M8 50 C 20 0, 40 0, 45 30 S 70 60, 82 10"
          fill="none"
          stroke="currentColor"
          strokeWidth={5}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
      </svg>
    </View>
  );
}
