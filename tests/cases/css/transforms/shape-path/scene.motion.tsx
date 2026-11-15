export const composition = { width: 460, height: 260, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  const route = "shape(from 10% 50%, curve to 90% 50% with 30% 0 / 70% 100%)";
  return (
    <View style={{ width: 460, height: 260, background: "white" }}>
      {[0, 1].map((row) => (
        <View
          key={`track-${row}`}
          style={{
            position: "absolute",
            left: 20,
            top: 20 + row * 120,
            width: 400 - row * 100 + seconds * 10,
            height: 100,
            border: "4px solid #cbd5e1",
            padding: 10,
            background: "#f1f5f9",
          }}
        >
          {[0, 25, 50, 75, 100].map((distance) => (
            <View
              style={{
                position: "absolute",
                width: 14,
                height: 10,
                background: row ? "#c026d3" : "#2563eb",
                offsetPath: `${route} ${row ? "content-box" : "border-box"}`,
                offsetDistance: `${distance + seconds * 12}%`,
                offsetRotate: "auto",
                offsetAnchor: "center",
              }}
            />
          ))}
        </View>
      ))}
    </View>
  );
}
