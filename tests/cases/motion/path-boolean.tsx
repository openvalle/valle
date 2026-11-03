export const composition = { width: 256, height: 96, fps: 4, duration: 1 };

// Shapes made from others: a star grown and shrunk with round joins, two circles sliding through
// each other in xor, and a square with a moving circle cut out of it.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const star = "M48 16 L56 40 L80 40 L60 54 L68 78 L48 64 L28 78 L36 54 L16 40 L40 40 Z";
  const circle = (x, y, r) =>
    `M${x - r} ${y} A${r} ${r} 0 1 1 ${x + r} ${y} A${r} ${r} 0 1 1 ${x - r} ${y} Z`;
  return (
    <svg width={256} height={96}>
      <path d={inflatePath(star, seconds * 12 - 4, { join: "round" })} fill="#fb8c00" />
      <path d={star} fill="none" stroke="#263238" strokeWidth={1} />
      <path
        d={pathBoolean(
          circle(112 + seconds * 24, 48, 24),
          circle(152 - seconds * 24, 48, 24),
          "xor",
        )}
        fill="#1e88e5"
      />
      <path
        d={pathBoolean("M184 16 H248 V80 H184 Z", circle(196 + seconds * 40, 48, 18), "subtract")}
        fill="#43a047"
      />
    </svg>
  );
}
