export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <linearGradient id="paint">
            <stop offset="0" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#ffb74d" />
            <stop offset="1" stopColor="#ffb74d" />
          </linearGradient>
          <clipPath id="clip">
            <rect x="10" y="20" width="120" height="100" />
          </clipPath>
          <mask id="mask" maskUnits="userSpaceOnUse" x="0" y="0" width="140" height="140">
            <rect width="70" height="140" fill="white" />
          </mask>
        </defs>
        <g transform="translate(20 20)" clipPath="url(#clip)" mask="url(#mask)">
          <rect width="140" height="140" fill="url(#paint)" />
          <rect x="40" y="40" width="80" height="60" fill="#16a085" />
        </g>
        <g transform="translate(180 20)" clipPath="url(#clip)" mask="url(#mask)" opacity="0.5">
          <rect width="140" height="140" fill="url(#paint)" />
          <rect x="40" y="40" width="80" height="60" fill="#16a085" />
        </g>
      </svg>
    </View>
  );
}
