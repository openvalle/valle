// error: errors/svg-text.tsx:7:7: TypeError: <text> in an <svg> is not supported yet
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <svg width={32} height={32}>
      <text x={1} y={8}>
        Hi
      </text>
    </svg>
  );
}
