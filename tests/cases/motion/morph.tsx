export const composition = { width: 256, height: 96, fps: 4, duration: 1 };

// Shapes becoming others: a star into a circle, a square into a triangle of the same commands,
// two dots into one bar, and a wave into a line.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const star = "M48 8 L58 38 L88 38 L64 56 L72 86 L48 68 L24 86 L32 56 L8 38 L38 38 Z";
  const circle = "M8 48 A40 40 0 1 1 88 48 A40 40 0 1 1 8 48 Z";
  return (
    <svg width={256} height={96}>
      <path d={morph(star, circle, seconds)} fill="#fb8c00" />
      <path
        d={morph("M104 16 L152 16 L152 64 L104 64 Z", "M128 8 L160 72 L96 72 L128 8 Z", seconds)}
        fill="#1e88e5"
      />
      <path
        d={morph(
          "M172 20 h12 v12 h-12 Z M204 20 h12 v12 h-12 Z",
          "M172 20 h44 v12 h-44 Z",
          seconds,
        )}
        fill="#8e24aa"
      />
      <path
        d={morph("M168 72 Q180 52 192 72 T216 72 T240 72", "M168 80 L240 64", seconds)}
        fill="none"
        stroke="#43a047"
        strokeWidth={3}
      />
    </svg>
  );
}
