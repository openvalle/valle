export const composition = { width: 208, height: 148, fps: 4, duration: 2 };
const STATES = [
  { width: 2, scale: 1, vector: "none", order: "normal" },
  { width: 2, scale: 1, vector: "non-scaling-stroke", order: "stroke markers fill" },
  { width: 2, scale: 1.5, vector: "none", order: "normal" },
  { width: 2, scale: 1.5, vector: "non-scaling-stroke", order: "stroke markers fill" },
  { width: 5, scale: 1, vector: "none", order: "normal" },
  { width: 5, scale: 1, vector: "non-scaling-stroke", order: "stroke markers fill" },
  { width: 5, scale: 1.5, vector: "none", order: "normal" },
  { width: 5, scale: 1.5, vector: "non-scaling-stroke", order: "stroke markers fill" },
];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const c = STATES[Math.floor(seconds * 4)];
  return (
    <View style={{ width: 208, height: 148, background: "white" }}>
      <svg width="208" height="148">
        <defs>
          <marker
            id="m"
            markerWidth="5"
            markerHeight="5"
            refX="4"
            refY="2.5"
            orient="auto-start-reverse"
          >
            <path d="M0 0L5 2.5L0 5Z" fill="#c05080" />
          </marker>
        </defs>
        <g transform="translate(8 8)">
          <g transform={`translate(12 12) scale(${c.scale})`}>
            <path
              d="M8 52 Q42 0 84 52 L8 52Z"
              fill="#e0b060"
              stroke="#2463a0"
              strokeWidth={c.width}
              strokeDasharray="8 4"
              strokeLinejoin="round"
              markerStart="url(#m)"
              markerEnd="url(#m)"
              vectorEffect={c.vector}
              paintOrder={c.order}
            />
          </g>
        </g>
      </svg>
    </View>
  );
}
