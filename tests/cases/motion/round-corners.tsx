export const composition = { width: 256, height: 112, fps: 4, duration: 1 };

// Corners rounded as the radius grows: a star, a square past the radius its sides allow, and an
// open zigzag with a curve in it.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const r = seconds * 24;
  const star = "M48 8 L58 38 L88 38 L64 56 L72 86 L48 68 L24 86 L32 56 L8 38 L38 38 Z";
  return (
    <svg width={256} height={112}>
      <path d={roundCorners(star, r / 3)} fill="#fb8c00" transform="translate(0 12)" />
      <path d={roundCorners("M104 16 H184 V96 H104 Z", r * 2)} fill="#1e88e5" />
      <path
        d={roundCorners("M196 96 L212 24 Q228 96 236 40 L248 96", r)}
        fill="none"
        stroke="#43a047"
        strokeWidth={3}
      />
    </svg>
  );
}
