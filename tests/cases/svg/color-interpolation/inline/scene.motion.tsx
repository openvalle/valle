export const composition = { width: 320, height: 160, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 320, height: 160, backgroundColor: "white" }}>
      <svg width="320" height="160">
        <defs>
          <linearGradient id="s">
            <stop stopColor="black" />
            <stop offset="1" stopColor="white" />
          </linearGradient>
          <linearGradient id="l" href="#s" colorInterpolation="linearRGB" />
          <radialGradient id="r" colorInterpolation="linearRGB">
            <stop stopColor="#204080" />
            <stop offset="1" stopColor="#e06020" stopOpacity="0.2" />
          </radialGradient>
          <g colorInterpolation="linearRGB">
            <linearGradient id="a">
              <stop stopColor="#204080" />
              <stop offset="1" stopColor="#e06020" stopOpacity="0.2" />
            </linearGradient>
            <linearGradient id="b" href="#a" style={{ colorInterpolation: "sRGB" }} />
          </g>
        </defs>
        <rect x="10" y="10" width="140" height="40" fill="url(#s)" />
        <rect x="170" y="10" width="140" height="40" fill="url(#l)" />
        <rect x="10" y="70" width="90" height="70" fill="url(#r)" />
        <rect x="115" y="70" width="90" height="70" fill="url(#a)" />
        <rect x="220" y="70" width="90" height="70" fill="url(#b)" />
      </svg>
    </View>
  );
}
