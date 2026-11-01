export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Overflow: rounded and square clips of what overflows, one axis clipped, absolute boxes inside
// and outside a clip, a fixed box escaping every clip, a layer under a clip, and a flex item that
// shrinks below its content once it clips.
export default function Scene() {
  const label = (text: string) => <Text style={{ fontSize: 10 }}>{text}</Text>;
  const band = (background: string, style: object) => <View style={{ background, ...style }} />;
  return (
    <View style={{ padding: 8 }}>
      <View style={{ display: "flex", gap: 8, marginBottom: 8 }}>
        <View
          style={{
            overflow: "hidden",
            width: 80,
            height: 40,
            borderRadius: 12,
            background: "#e3f2fd",
          }}
        >
          {band("#1565c0", { height: 16, marginLeft: -10, width: 120 })}
          {label("Rounded clip of text that runs on")}
        </View>
        <View style={{ overflow: "clip", width: 60, height: 40, background: "#e8f5e9" }}>
          {band("#2e7d32", { height: 30, width: 90, marginTop: 20 })}
        </View>
        <View style={{ overflowX: "clip", width: 60, height: 20, background: "#fff3e0" }}>
          {band("#ef6c00", { height: 36, width: 90 })}
        </View>
      </View>
      <View style={{ position: "relative", height: 50, marginBottom: 8, background: "#f3e5f5" }}>
        <View
          style={{
            position: "relative",
            overflow: "hidden",
            width: 60,
            height: 30,
            background: "#ce93d8",
          }}
        >
          {band("#6a1b9a", { position: "absolute", left: 40, top: 15, width: 40, height: 30 })}
        </View>
        <View
          style={{
            overflow: "hidden",
            width: 60,
            height: 16,
            marginLeft: 100,
            marginTop: -30,
            background: "#ba68c8",
          }}
        >
          {band("#4a148c", { position: "absolute", left: 140, top: 20, width: 40, height: 26 })}
          {band("#7b1fa2", {
            position: "fixed",
            right: 8,
            bottom: 8,
            width: 24,
            height: 24,
          })}
        </View>
      </View>
      <View
        style={{
          overflow: "hidden",
          height: 24,
          borderRadius: 6,
          marginBottom: 8,
          background: "#fce4ec",
        }}
      >
        <View style={{ opacity: 0.6, height: 40, background: "#d81b60" }}>
          {band("#880e4f", { position: "absolute", left: 120, top: 150, width: 30, height: 30 })}
        </View>
      </View>
      <View style={{ display: "flex", width: 120, background: "#e0f2f1" }}>
        <View style={{ overflow: "hidden", height: 12, background: "#4db6ac" }}>
          {band("#00695c", { width: 150, height: 6 })}
        </View>
        <View style={{ height: 12, background: "#80cbc4" }}>
          {band("#004d40", { width: 150, height: 6 })}
        </View>
      </View>
    </View>
  );
}
