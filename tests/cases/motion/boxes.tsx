export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Backgrounds, corner radii and edges between pixels.
export default function Scene() {
  const box = { width: 64, height: 64 };
  return (
    <View
      style={{
        height: 256,
        padding: 16,
        background: "white",
        display: "flex",
        flexWrap: "wrap",
        gap: 16,
      }}
    >
      <View style={{ ...box, background: "#e53935" }} />
      <View style={{ ...box, background: "#1e88e5", borderRadius: 12 }} />
      <View style={{ ...box, background: "#43a047", borderRadius: 999 }} />
      <View style={{ width: 100, height: 40, background: "#fdd835", borderRadius: 20 }} />
      <View style={{ width: 40.5, height: 40.25, background: "#8e24aa" }} />
      <View style={{ width: 1.5, height: 64, background: "black" }} />
      <View style={{ ...box, background: "rgb(0 0 0 / 50%)", borderRadius: 8 }} />
      <View style={{ ...box, background: "hsl(200 80% 50%)", borderRadius: 32 }} />
    </View>
  );
}
