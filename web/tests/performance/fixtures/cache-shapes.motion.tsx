export const composition = { width: 640, height: 360, fps: 60, duration: 10 };

const star = path("M 60 0 L 74.11 40.58 L 117.06 41.46 L 82.83 67.42 L 95.27 108.54 L 60 84 L 24.73 108.54 L 37.17 67.42 L 2.94 41.46 L 45.89 40.58 Z");

// Blurred shadows and concave paths that drift by subpixels; GPU mask and blur caches key on them.
export default function CacheShapes() { const time = useCurrentTime(); const frame = Math.floor(time.seconds * 60);
  const drift = frame * 0.37;
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#e2e8f0" }}>
      <View key="card-a" style={{ position: "absolute", left: 40 + drift, top: 60 + drift * 0.5, width: 160, height: 100,
        borderRadius: 18, backgroundColor: "#ffffff", boxShadow: "0px 12px 28px #0f172a66" }} />
      <View key="card-b" style={{ position: "absolute", left: 260 - drift * 0.6, top: 200 - drift * 0.3, width: 140, height: 90,
        borderRadius: 24, backgroundColor: "#38bdf8", boxShadow: "0px 6px 16px #0f172a80" }} />
      <View key="star" style={{ position: "absolute", left: 440 + drift * 0.4, top: 90 + drift * 0.2, width: 120, height: 120,
        transform: `rotate(${frame * 0.3}deg)`, filter: "drop-shadow(4px 6px 5px #0f172a99)" }}>
        <Path d={star} fill="#f472b6" />
      </View>
    </Scene>
  );
}
