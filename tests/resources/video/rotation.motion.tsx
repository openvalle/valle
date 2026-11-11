export const composition = { width: 560, height: 144, fps: 4, duration: 1 };
export const controls = {
  resources: {
    original: resource({ kind: "video", default: "quadrants.mp4" }),
    left: resource({ kind: "video", default: "rotate90.mp4" }),
    upside: resource({ kind: "video", default: "rotate180.mp4" }),
    right: resource({ kind: "video", default: "rotate270.mp4" }),
    mirrored: resource({ kind: "video", default: "mirror90.mp4" }),
  },
};

export default function Scene() {
  return (
    <View style={{ width: "100%", height: "100%", backgroundColor: "#eeeeee" }}>
      {["original", "left", "upside", "right", "mirrored"].map((name, i) => (
        <View key={name} style={{ position: "absolute", left: 8 + 112 * i, top: 16 }}>
          <Video
            src={`resource://${name}`}
            style={{ width: 96, height: 96, objectFit: "contain" }}
          />
          <Text style={{ fontSize: 12, marginTop: 4 }}>{name}</Text>
        </View>
      ))}
    </View>
  );
}
