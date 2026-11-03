// error: errors/svg-animate.tsx:7:7: TypeError: <animate> is not supported: animate attributes with interpolate() in their values
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <svg width={32} height={32}>
      <animate />
    </svg>
  );
}
