export const composition = { width: 256, height: 96, fps: 4, duration: 1 };

// AE's path modifiers growing: a square bloated, a star puckered, a bar twisted, and a circle and
// a line zigzagged.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const square = "M8 24 H56 V72 H8 Z";
  const star = "M88 16 L98 40 L124 40 L104 56 L112 82 L88 66 L64 82 L72 56 L52 40 L78 40 Z";
  const circle = "M200 48 A24 24 0 1 1 152 48 A24 24 0 1 1 200 48 Z";
  return (
    <svg width={256} height={96}>
      <path d={puckerBloat(square, seconds * 0.4)} fill="#1e88e5" />
      <path d={puckerBloat(star, -seconds * 0.3)} fill="#fb8c00" />
      <path d={twist("M132 40 H148 V56 H132 Z", seconds * 180)} fill="#8e24aa" />
      <path
        d={zigzag(circle, { size: seconds * 4, ridges: 5 })}
        fill="none"
        stroke="#43a047"
        strokeWidth={2}
      />
      <path
        d={zigzag("M212 80 L248 16", { size: 3, ridges: 6 })}
        fill="none"
        stroke="#e53935"
        strokeWidth={2}
      />
    </svg>
  );
}
