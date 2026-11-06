export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <linearGradient id="linear">
            <stop offset="0" stopColor="#000000" />
            <stop offset="1" stopColor="#ffffff" />
          </linearGradient>
          <linearGradient id="hard" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0.5" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#ffb74d" />
          </linearGradient>
          <radialGradient id="radial">
            <stop offset="0" stopColor="#ffffff" />
            <stop offset="1" stopColor="#176bba" />
          </radialGradient>
        </defs>
        <rect x="20" y="20" width="90" height="120" fill="url(#linear)" />
        <rect x="125" y="20" width="90" height="120" fill="url(#hard)" />
        <rect x="230" y="20" width="90" height="120" fill="url(#radial)" />
      </svg>
    </View>
  );
}
