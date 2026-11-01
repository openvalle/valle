export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// A translucent group shows its overlapping children once; translucent children each show.
export default function Scene() {
  const square = (left: number, top: number, background: string) => (
    <View style={{ position: "absolute", left, top, width: 64, height: 64, background }} />
  );
  return (
    <View style={{ width: 256, height: 256, background: "#f5f5f5" }}>
      <View style={{ position: "absolute", left: 0, top: 0, opacity: 0.5 }}>
        {square(24, 24, "#d81b60")}
        {square(56, 56, "#1e88e5")}
      </View>
      <View style={{ position: "absolute", left: 128, top: 0 }}>
        <View style={{ opacity: 0.5 }}>{square(24, 24, "#d81b60")}</View>
        <View style={{ opacity: 0.5 }}>{square(56, 56, "#1e88e5")}</View>
      </View>
      <View style={{ position: "absolute", left: 0, top: 128, opacity: 0.5 }}>
        <View style={{ opacity: 0.5 }}>{square(24, 24, "black")}</View>
      </View>
      <View style={{ position: "absolute", left: 128, top: 128, opacity: 0 }}>
        {square(24, 24, "black")}
      </View>
    </View>
  );
}
