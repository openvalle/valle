export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <linearGradient id="base">
            <stop offset="0" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#ffb74d" />
            <stop offset="1" stopColor="#ffb74d" />
          </linearGradient>
          <linearGradient id="vertical" href="#base" x1="0" y1="0" x2="0" y2="1" />
          <linearGradient
            id="leaf"
            href="#vertical"
            gradientTransform="translate(0 1) scale(1 -1)"
          />
          <linearGradient id="own" href="#leaf">
            <stop offset="0" stopColor="#16a085" />
            <stop offset="1" stopColor="#16a085" />
          </linearGradient>
        </defs>
        <rect x="20" y="20" width="80" height="140" fill="url(#base)" />
        <rect x="120" y="20" width="80" height="140" fill="url(#leaf)" />
        <rect x="220" y="20" width="80" height="140" fill="url(#own)" />
      </svg>
    </View>
  );
}
