export const composition = { width: 320, height: 200, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 320, height: 200, backgroundColor: "white" }}>
      <svg xmlns="http://www.w3.org/2000/svg" width="320" height="200" viewBox="0 0 160 100">
        <defs>
          <linearGradient id="ink">
            <stop stopColor="#3366cc" />
            <stop offset="1" stopColor="#dd3355" />
          </linearGradient>
          <path
            id="line"
            d="M0 0L30 0"
            fill="none"
            stroke="#008977"
            strokeWidth="4"
            vectorEffect="non-scaling-stroke"
          />
        </defs>
        <g fill="none" stroke="#254566" strokeWidth="4">
          <path d="M10 12L65 12L65 30" />
          <path d="M90 12L145 12L145 30" vectorEffect="non-scaling-stroke" />
          <g transform={`translate(10 50) scale(${1.7 + seconds * 0.2} .65)`}>
            <path
              d="M0 0C10 -25 25 25 36 0"
              stroke="url(#ink)"
              vectorEffect="non-scaling-stroke"
              strokeLinecap="round"
              strokeDasharray="6 4"
            />
          </g>
          <path
            d="M90 44L145 44"
            vectorEffect="non-scaling-stroke"
            strokeDasharray="8 4"
            pathLength="55"
          />
          <path
            d="M90 55L145 55"
            vectorEffect="non-scaling-stroke"
            strokeLinecap="round"
            strokeDasharray="0 9"
          />
          <g vectorEffect="non-scaling-stroke">
            <path d="M10 77L65 77" />
            <path d="M10 89L65 89" style={{ vectorEffect: "non-scaling-stroke" }} />
          </g>
          <use href="#line" x="95" y="78" transform="skewX(8)" />
          <path
            d="M95 90L145 90"
            vectorEffect="non-scaling-stroke"
            style={{ vectorEffect: "none" }}
          />
        </g>
      </svg>
    </View>
  );
}
