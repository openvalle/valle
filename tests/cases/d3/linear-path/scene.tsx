export const composition = { width: 240, height: 140, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <svg key="plot" width={120} height={100}>
      <path
        d={line()([
          [10, 20],
          [50, 60],
          [90, 20],
        ])}
        fill="none"
        stroke="blue"
        strokeWidth={2}
      />
    </svg>
  );
}
