export const composition = { width: 320, height: 128, fps: 4, duration: 1 };

const style = { fontSize: 56, fontWeight: 800 };
const title = textOutline("Outline", { ...style, origin: { x: 160, y: 72 }, align: "center" });
const { x, y, width, height } = title.bounds;

// A word as paths: its outline stroked, its glyphs filling in one after another, its bounds
// boxed.
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <svg width={320} height={128} style={{ background: "#e3f2fd" }}>
      <rect
        x={x - 4}
        y={y - 4}
        width={width + 8}
        height={height + 8}
        fill="none"
        stroke="#90caf9"
        strokeDasharray="4 4"
      />
      {title.glyphs.map((g, i) => (
        <path d={g.path} fill="#3949ab" opacity={clamp(seconds * 8 - i, 0, 1)} />
      ))}
      <path d={title.path} fill="none" stroke="#e91e63" strokeWidth={1.5} />
    </svg>
  );
}
