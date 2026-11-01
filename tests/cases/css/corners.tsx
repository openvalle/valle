export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Corners: circles, ellipses from percentages and slashes, one corner at a time, logical corners,
// radii too large for their box shrinking together, and an elliptical clip.
export default function Scene() {
  const tile = (background: string, style: object) => (
    <View style={{ width: 52, height: 36, background, ...style }} />
  );
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {tile("#e53935", { borderRadius: "50%" })}
      {tile("#d81b60", { borderRadius: "40px / 12px" })}
      {tile("#8e24aa", { borderRadius: "24px 4px" })}
      {tile("#5e35b1", { borderRadius: "10px 20px 30px 0 / 4px 12px" })}
      {tile("#3949ab", { borderTopLeftRadius: 18, borderBottomRightRadius: "50% 30%" })}
      {tile("#1e88e5", { borderStartEndRadius: "30%", borderEndStartRadius: 14 })}
      {tile("#00897b", { borderRadius: 200 })}
      {tile("#43a047", { borderRadius: "80px 10px 40px 20px" })}
      {tile("#fdd835", { width: 100, borderRadius: "calc(50% - 10px) / 100%" })}
      <View
        style={{
          overflow: "hidden",
          width: 112,
          height: 60,
          borderRadius: "50% 50% 20% 20% / 60% 60% 40% 40%",
          background: "#fff3e0",
        }}
      >
        <View style={{ height: 30, background: "#fb8c00" }} />
        <View style={{ height: 30, marginLeft: -10, width: 140, background: "#e65100" }} />
      </View>
      <View style={{ display: "flex", gap: 4 }}>
        {[2, 4, 8, 16].map((r) => tile("#6d4c41", { width: 24, height: 60, borderRadius: r }))}
      </View>
    </View>
  );
}
