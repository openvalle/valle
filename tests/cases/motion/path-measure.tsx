export const composition = { width: 256, height: 128, fps: 4, duration: 1 };

const track = "M16 96 C64 0 128 160 176 48 S232 32 240 96";

// An arrow riding a curve, turned along it, and a bar as long as a tenth of the curve.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const p = pointAt(track, seconds);
  const { angle } = tangentAt(track, seconds);
  return (
    <View style={{ width: 256, height: 128 }}>
      <svg width={256} height={128} style={{ position: "absolute", left: 0, top: 0 }}>
        <path d={track} fill="none" stroke="#b0bec5" strokeWidth={2} />
      </svg>
      <View
        style={{
          position: "absolute",
          left: p.x - 10,
          top: p.y - 6,
          width: 20,
          height: 12,
          background: "#e53935",
          clipPath: "polygon(0 0, 100% 50%, 0 100%)",
          rotate: `${angle}deg`,
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 8,
          top: 112,
          height: 8,
          width: pathLength(track) / 10,
          background: "#1e88e5",
        }}
      />
    </View>
  );
}
