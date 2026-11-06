export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <radialGradient id="center">
            <stop offset="0" stopColor="#fff" />
            <stop offset="1" stopColor="#176bba" />
          </radialGradient>
          <radialGradient id="focus" href="#center" fx="0.25" fy="0.35" />
          <radialGradient id="inner" href="#center" fx="0.4" fy="0.5" fr="0.15" />
        </defs>
        <rect x="20" y="20" width="90" height="140" fill="url(#center)" />
        <rect x="125" y="20" width="90" height="140" fill="url(#focus)" />
        <rect x="230" y="20" width="90" height="140" fill="url(#inner)" />
      </svg>
    </View>
  );
}
