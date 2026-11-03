export const composition = { width: 320, height: 120, fps: 4, duration: 1 };

export const controls = {
  resources: {
    clip: resource({ kind: "video", default: "../resources/counter.mp4" }),
    again: resource({ kind: "video", default: "../resources/counter.mp4" }),
  },
};

// Videos: one from 0.5 s in at twice the speed, cover-fitted and rounded; the same file again
// held on its frame at 1.25 s, turned. Each shows the frame starting at or before its time.
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View
      style={{
        display: "flex",
        gap: 16,
        padding: 12,
        width: 320,
        height: 120,
        background: "#eceff1",
      }}
    >
      <Video
        src="resource://clip"
        trimStart={0.5}
        rate={2}
        style={{ width: 160, height: 96, objectFit: "cover", borderRadius: 12 }}
      />
      <Video
        src="resource://again"
        time={1.25}
        style={{ width: 96, height: 72, transform: `rotate(${seconds * 20}deg)` }}
      />
    </View>
  );
}
