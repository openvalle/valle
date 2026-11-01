export const composition = { width: 360, height: 144, fps: 1, duration: 1 };

// SVG markers: an arrow at a line's end, and at a polyline's start turned back, scaled by the
// stroke width; squares at every vertex of a polygon by the marker shorthand, turned between the
// sides; chevrons along curves and an arc, each command one vertex; a marker's content clipped to
// its viewport, filled as the marker's own parent says, at a fixed angle, from a group's
// markerEnd; and dots in the middle of a polyline in a turned, translucent group.
export default function Scene() {
  const box = { width: 100, height: 60 };
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8, background: "#eceff1" }}>
      <svg {...box}>
        <marker
          id="k1"
          viewBox="0 0 10 10"
          refX={5}
          refY={5}
          markerWidth={4}
          markerHeight={4}
          orient="auto-start-reverse"
        >
          <path d="M0 0 L10 5 L0 10 Z" fill="#e53935" />
        </marker>
        <line
          x1={10}
          y1={12}
          x2={86}
          y2={12}
          stroke="#263238"
          strokeWidth={1.5}
          markerEnd="url(#k1)"
        />
        <polyline
          points="12,48 35,28 60,48 84,28"
          fill="none"
          stroke="#3949ab"
          strokeWidth={2.5}
          markerStart="url(#k1)"
          markerEnd="url(#k1)"
        />
      </svg>
      <svg {...box}>
        <marker
          id="k2"
          markerWidth={6}
          markerHeight={6}
          refX={3}
          refY={3}
          markerUnits="userSpaceOnUse"
          orient="auto"
        >
          <rect width={6} height={6} fill="#43a047" />
        </marker>
        <polygon
          points="20,10 80,14 70,50 30,46"
          fill="#e3f2fd"
          stroke="#1e88e5"
          strokeWidth={2}
          style={{ marker: "url(#k2)" }}
        />
      </svg>
      <svg {...box}>
        <marker
          id="k3"
          viewBox="-5 -5 10 10"
          markerWidth={8}
          markerHeight={8}
          markerUnits="userSpaceOnUse"
          orient="auto"
        >
          <path d="M-4 -3 L4 0 L-4 3" fill="none" stroke="#8e24aa" strokeWidth={1.5} />
        </marker>
        <path
          d="M10 40 C20 10, 40 10, 50 30 S80 50, 90 20 A12 12 0 0 1 70 10"
          fill="none"
          stroke="#90a4ae"
          strokeWidth={1.5}
          markerStart="url(#k3)"
          markerMid="url(#k3)"
          markerEnd="url(#k3)"
        />
      </svg>
      <svg {...box}>
        <g fill="#fb8c00">
          <marker
            id="k4"
            viewBox="0 0 10 10"
            refX={5}
            refY={5}
            markerWidth={14}
            markerHeight={14}
            markerUnits="userSpaceOnUse"
            orient="30"
          >
            <circle cx={5} cy={5} r={7} />
          </marker>
        </g>
        <g markerEnd="url(#k4)" stroke="#6d4c41" strokeWidth={2} fill="none">
          <line x1={10} y1={30} x2={36} y2={30} />
          <path d="M52 46 Q70 6 86 40" />
        </g>
      </svg>
      <svg {...box}>
        <marker id="k5" markerWidth={3} markerHeight={3} refX={1.5} refY={1.5}>
          <circle cx={1.5} cy={1.5} r={1.5} fill="#fdd835" />
        </marker>
        <g transform="translate(50 30) rotate(-20)" opacity={0.7}>
          <polyline
            points="-35,10 -12,-10 12,10 35,-10"
            fill="none"
            stroke="#00897b"
            strokeWidth={3}
            markerMid="url(#k5)"
          />
        </g>
      </svg>
    </View>
  );
}
