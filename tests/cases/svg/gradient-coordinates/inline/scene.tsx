export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <linearGradient id="user" gradientUnits="userSpaceOnUse" x1="20" y1="0" x2="300" y2="0">
            <stop offset="0" stopColor="#176bba" />
            <stop offset="1" stopColor="#ffb74d" />
          </linearGradient>
          <linearGradient id="box">
            <stop offset="0" stopColor="#176bba" />
            <stop offset="1" stopColor="#ffb74d" />
          </linearGradient>
          <linearGradient id="flip" href="#box" gradientTransform="translate(1 0) scale(-1 1)" />
        </defs>
        <rect x="20" y="20" width="80" height="40" fill="url(#user)" />
        <rect x="140" y="20" width="160" height="40" fill="url(#user)" />
        <rect x="20" y="80" width="80" height="40" fill="url(#box)" />
        <rect x="140" y="80" width="160" height="40" fill="url(#box)" />
        <rect x="20" y="140" width="280" height="30" fill="url(#flip)" />
      </svg>
    </View>
  );
}
