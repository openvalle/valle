// error: errors/svg-foreign.tsx:7:7: TypeError: <foreignObject> is not supported: put HTML-like content in Valle elements outside the <svg>
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <svg width={32} height={32}>
      <foreignObject />
    </svg>
  );
}
