export const composition = { width: 380, height: 230, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 380, height: 230, background: "white" }}>
      <svg width="380" height="230" fontFamily="sans-serif">
        <defs>
          <linearGradient id="ink">
            <stop stopColor="#2563eb" />
            <stop offset="1" stopColor="#e11d48" />
          </linearGradient>
          <path id="ring" d="M0 0H60V60H0Z M15 15H45V45H15Z" />
          <text id="label" x="0" y="0">
            Use
          </text>
          <clipPath id="letters">
            <text
              x={20 + seconds * 15}
              y="75"
              fontSize="62"
              fontWeight="700"
              fill="none"
              stroke="red"
              strokeWidth="10"
              opacity="0"
            >
              VA<tspan dx="3">LLE</tspan>
            </text>
          </clipPath>
          <clipPath id="reused">
            <use href="#ring" x="20" y="110" clipRule="evenodd" transform="rotate(8 50 140)" />
            <use href="#label" x="100" y="160" fontSize="52" />
          </clipPath>
          <clipPath id="normalized" clipPathUnits="objectBoundingBox">
            <text x="0.1" y="0.8" fontSize="0.8">
              A
            </text>
          </clipPath>
        </defs>
        <rect width="380" height="95" fill="url(#ink)" clipPath="url(#letters)" />
        <rect y="95" width="250" height="110" fill="#0d9488" clipPath="url(#reused)" />
        <rect x="265" y="105" width="95" height="95" fill="#f97316" clipPath="url(#normalized)" />
      </svg>
    </View>
  );
}
