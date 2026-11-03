// error: errors/svg-nested.tsx:7:7: TypeError: an <svg> inside an <svg> is not supported yet
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <svg width={32} height={32}>
      <svg width={8} height={8} />
    </svg>
  );
}
