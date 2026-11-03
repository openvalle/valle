export const composition = { width: 360, height: 240, fps: 1, duration: 1 };

// Path data in its many spellings: every command absolute and relative; repeated commands without
// their letters, and moves that go on as lines; numbers run together; the four arcs between two
// points, by their flags, the flags without spaces; smooth curves that follow a curve, and ones
// that follow none; a subpath started after a close, without a move; holes by winding and
// even-odd, filled and stroked; and arcs with radii too small, which grow, and of radius zero.
export default function Scene() {
  const box = { width: 80, height: 60, style: { background: "#eceff1" } };
  const fill = { fill: "#90caf9", stroke: "#0d47a1", strokeWidth: 1.5 };
  const open = { fill: "none", stroke: "#e53935", strokeWidth: 1.5 };
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8 }}>
      <svg {...box}>
        <path
          {...fill}
          d="M8 8 L24 8 H40 V24 L56 24 Q72 24 72 40 C72 52 48 52 40 52 A8 8 0 0 1 24 52 Z"
        />
      </svg>
      <svg {...box}>
        <path
          {...fill}
          d="m8 8 l16 0 h16 v16 l16 0 q16 0 16 16 c0 12-24 12-32 12 a8 8 0 0 1-16 0 z"
        />
      </svg>
      <svg {...box}>
        <path {...open} d="M8 50 20 10 32 50 44 10 56 50 68 10" />
      </svg>
      <svg {...box}>
        <path {...open} d="M.5.5l10 10.5-.5 10.25 8-3 8.5.5.25 20h10v-8.5.5" />
      </svg>
      <svg {...box}>
        <path {...open} d="M20 40 A20 14 0 0 0 60 20" />
        <path {...open} stroke="#1e88e5" d="M20 40 A20 14 0 0 1 60 20" />
        <path {...open} stroke="#43a047" d="M20 40 A20 14 0 1 0 60 20" />
        <path {...open} stroke="#fb8c00" d="M20 40 A20 14 0 1 1 60 20" />
      </svg>
      <svg {...box}>
        <path {...open} d="M10 30a15 15 0 0130 0a15 15 0 1030 0" />
      </svg>
      <svg {...box}>
        <path {...open} d="M8 30 C20 4 30 4 40 30 S60 56 72 30" />
        <path {...open} stroke="#1e88e5" d="M8 50 Q24 20 40 50 T72 50" />
        <path {...open} stroke="#43a047" d="M8 10 S20 30 40 10 T72 10" />
      </svg>
      <svg {...box}>
        <path {...fill} d="M8 8 H36 V30 H8 Z L24 52 H48 Z M56 8 H72 V52 H56 Z" />
      </svg>
      <svg {...box}>
        <path {...fill} d="M8 8 H72 V52 H8 Z M24 20 H56 V40 H24 Z" />
      </svg>
      <svg {...box}>
        <path {...fill} fillRule="evenodd" d="M8 8 H72 V52 H8 Z M24 20 H56 V40 H24 Z" />
      </svg>
      <svg {...box}>
        <path {...fill} d="M8 8 H72 V52 H8 Z M24 20 V40 H56 V20 Z" />
      </svg>
      <svg {...box}>
        <path {...open} d="M10 30 A4 4 0 0 1 70 30" />
        <path {...open} stroke="#1e88e5" d="M10 50 A0 10 0 0 1 70 50" />
        <path {...open} stroke="#43a047" d="M10 10 A40 20 30 0 1 70 20" />
      </svg>
    </View>
  );
}
