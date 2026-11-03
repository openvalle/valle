// error: errors/svg-pattern.tsx:7:7: TypeError: <pattern> in an <svg> is not supported yet
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <svg width={32} height={32}>
      <pattern id="p" />
    </svg>
  );
}
