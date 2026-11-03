// error: errors/removed-function.tsx:6: TypeError: morphPath is removed: use morph()…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const { progress } = useCurrentTime();
  const d = morphPath("M0 0 H10", "M0 10 H10", progress);
  return (
    <svg width={64} height={64}>
      <path d={d} stroke="black" />
    </svg>
  );
}
