export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Radial gradients: an ellipse to the farthest corner by default, circles and ellipses to the
// closest or farthest side or corner from a center anywhere, explicit radii with hard stops,
// repeating rings, mixed in Oklch, a hint, layers over a color, repeated under a transparent
// border, and in a layer.
export default function Scene() {
  const tile = (background: string, style: object = {}) => (
    <View style={{ width: 52, height: 36, background, ...style }} />
  );
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {tile("radial-gradient(#fff59d, #f57f17)")}
      {tile("radial-gradient(circle, #e1f5fe, #0277bd)")}
      {tile("radial-gradient(circle closest-side at 30% 40%, #fce4ec, #ad1457)")}
      {tile("radial-gradient(closest-corner at 10px 10px, #e8f5e9, #2e7d32)")}
      {tile("radial-gradient(farthest-side at right 8px bottom 25%, #ede7f6, #4527a0)")}
      {tile("radial-gradient(20px 10px, #ffccbc 0 40%, #bf360c 40% 70%, transparent 70%), #fbe9e7")}
      {tile("repeating-radial-gradient(circle at 0 0, #5d4037 0 4px, #d7ccc8 4px 8px)")}
      {tile("radial-gradient(circle in oklch, #e53935, #1e88e5)")}
      {tile("radial-gradient(ellipse 120% 80% at 50% 0, #80deea, #006064)", { width: 112 })}
      {tile("radial-gradient(circle 30px at 15px 50%, #ffffff, 20%, #212121)")}
      {tile("radial-gradient(circle, rgb(255 255 255 / 90%), transparent 60%), #3949ab")}
      {tile("radial-gradient(#ffecb3, #ff6f00)", {
        border: "6px solid transparent",
        borderRadius: 12,
      })}
      {tile("radial-gradient(circle at 70% 30%, #ffeb3b, #f44336)", { opacity: 0.6 })}
      {tile(
        "radial-gradient(circle at 25% 50%, #fff 0 6px, transparent 7px), radial-gradient(circle at 75% 50%, #fff 0 6px, transparent 7px), #26a69a",
      )}
    </View>
  );
}
