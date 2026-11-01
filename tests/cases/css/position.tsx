export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Relative, absolute and fixed positioning: offsets, insets that size a box, percentages, and which
// box contains an absolute one. A fixed box is placed on the canvas, out of positioned ancestors.
export default function Scene() {
  const box = (background: string, style: object) => (
    <View style={{ position: "absolute", background, ...style }} />
  );
  return (
    <View style={{ padding: 10 }}>
      <View style={{ position: "relative", height: 100, margin: 6, background: "#e3f2fd" }}>
        {box("#1565c0", { left: 4, top: 4, width: 40, height: 20 })}
        {box("#1976d2", { right: 4, bottom: 4, width: 40, height: 20 })}
        {box("#1e88e5", { left: 60, right: 60, top: 30, bottom: 30 })}
        {box("#42a5f5", { inset: "10% 5% auto auto", width: "20%", height: 10 })}
        {box("#64b5f6", { left: "50%", top: "50%", width: 16, height: 16 })}
      </View>
      <View style={{ height: 40, background: "#f3e5f5" }}>
        <View
          style={{
            position: "relative",
            left: 20,
            top: 10,
            width: 60,
            height: 20,
            background: "#8e24aa",
          }}
        />
        {box("#ab47bc", { right: 0, bottom: 0, width: 30, height: 30 })}
      </View>
      <View style={{ position: "relative", marginTop: 8, height: 50, background: "#fff3e0" }}>
        <View
          style={{ position: "relative", right: 10, bottom: 5, height: 20, background: "#ef6c00" }}
        >
          {box("#ffa726", { left: 0, right: 0, top: "100%", height: 6 })}
        </View>
        {box("#fb8c00", { inset: 12, left: "auto", width: 30 })}
        <View style={{ position: "fixed", width: 10, height: 10, background: "#43a047" }} />
        <View
          style={{
            position: "fixed",
            right: 4,
            bottom: 4,
            width: 24,
            height: 24,
            background: "#2e7d32",
          }}
        >
          {box("#a5d6a7", { left: 4, top: 4, width: 8, height: 8 })}
        </View>
        <View
          style={{
            position: "fixed",
            left: "75%",
            top: "5%",
            width: "10%",
            height: 8,
            background: "#66bb6a",
          }}
        />
      </View>
    </View>
  );
}
