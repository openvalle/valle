export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

// A flow chart whose links follow its boxes as layout moves them: a curve and a line between
// anchors, the line drawn on along its length, a ring around one box and a dot on the corner of
// another, all from the boxes as laid out.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const box = { width: 48, height: 32, borderRadius: 6 };
  const ab = connect(anchor("chart/a", "right"), anchor("chart/b", "left"), { route: "cubic" });
  const bc = connect(anchor("chart/b", "right"), anchor("chart/c", "left"));
  const length = pathLength(bc);
  const c = bounds("chart/c");
  const corner = anchor("chart/b", "top-right");
  return (
    <View style={{ width: 256, height: 160 }}>
      <View
        key="chart"
        style={{ display: "flex", alignItems: "center", gap: 32, padding: 24, height: 160 }}
      >
        <View key="a" style={{ ...box, background: "#1e88e5" }} />
        <View
          key="b"
          style={{
            ...box,
            background: "#43a047",
            marginTop: interpolate(seconds, [0, 1], [-60, 60]),
          }}
        />
        <View
          key="c"
          style={{ ...box, background: "#fb8c00", height: interpolate(seconds, [0, 1], [24, 56]) }}
        />
      </View>
      <svg width={256} height={160} style={{ position: "absolute", left: 0, top: 0 }}>
        <path d={ab} fill="none" stroke="#546e7a" strokeWidth={2} />
        <path
          d={bc}
          fill="none"
          stroke="#546e7a"
          strokeWidth={2}
          strokeDasharray={`${length} ${length}`}
          strokeDashoffset={length * (1 - seconds)}
        />
        <rect
          x={c.x - 4}
          y={c.y - 4}
          width={c.width + 8}
          height={c.height + 8}
          rx={8}
          fill="none"
          stroke="#e53935"
          strokeWidth={2}
        />
      </svg>
      <View
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          width: 12,
          height: 12,
          borderRadius: 6,
          background: "#8e24aa",
          translate: `${corner.x - 6}px ${corner.y - 6}px`,
        }}
      />
    </View>
  );
}
