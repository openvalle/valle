export const composition = { width: 610, height: 290, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 610, height: 290, background: "white" }}>
      <View
        key="bubble"
        style={{
          position: "absolute",
          left: 10,
          top: 10,
          width: 180 + seconds * 8,
          height: 115,
          background: "#2563eb",
          clipPath:
            "shape(from 12px 0, hline to calc(100% - 12px), curve to 100% 12px with 100% 0, vline to calc(100% - 32px), curve by -12px 12px with 0 12px, hline to 65%, line by -12px 20px, line by -12px -20px, hline to 12px, curve to 0 calc(100% - 32px) with 0 calc(100% - 20px), vline to 12px, curve to 12px 0 with 0 0, close)",
        }}
      />
      <View
        key="small"
        style={{
          position: "absolute",
          left: 210,
          top: 10,
          width: 130,
          height: 85 + seconds * 15,
          background: "#a855f7",
          clipPath:
            "shape(from 12px 0, hline to calc(100% - 12px), curve to 100% 12px with 100% 0, vline to calc(100% - 32px), curve by -12px 12px with 0 12px, hline to 65%, line by -12px 20px, line by -12px -20px, hline to 12px, curve to 0 calc(100% - 32px) with 0 calc(100% - 20px), vline to 12px, curve to 12px 0 with 0 0, close)",
        }}
      />
      <View
        key="hole"
        style={{
          position: "absolute",
          left: 410,
          top: 10,
          width: 180,
          height: 115,
          background: "#16a34a",
          clipPath:
            "shape(evenodd from 0 0, hline to right, vline to bottom, hline to left, close, move to 25% 25%, line to 75% 25%, line to 75% 75%, line to 25% 75%, close)",
        }}
      />
      <View
        key="wave"
        style={{
          position: "absolute",
          left: 10,
          top: 155,
          width: 180,
          height: 115,
          background: "#f97316",
          clipPath:
            "shape(from 0 50%, curve by 50% 0 with 15% -45% / 35% -45%, smooth by 50% 0 with -15% 45% from end, line to right bottom, line to left bottom, close)",
        }}
      />
      <View
        key="arc"
        style={{
          position: "absolute",
          left: 210,
          top: 145,
          width: 180,
          height: 125,
          background: "#0891b2",
          clipPath:
            "shape(from 20% 50%, arc to 80% 50% of 40% cw large rotate 15deg, arc to 20% 50% of 35% 20% ccw, close)",
          transform: "rotate(-5deg)",
        }}
      />
      <View
        key="content"
        style={{
          position: "absolute",
          left: 410,
          top: 155,
          width: 180,
          height: 115,
          padding: 12,
          border: "6px solid #b91c1c",
          background: "#dc2626",
          clipPath:
            "shape(from left top, curve to right center with 30% 10px from start / -10px -30% from end, smooth to left bottom with center bottom, close) content-box",
        }}
      />
    </View>
  );
}
