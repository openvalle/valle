export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Conic gradients: a color wheel, from an angle, quadrants around a center elsewhere, repeating
// rays, a full turn of hues in Oklch, a hint, pie slices with stops at 0, a shadow from a corner
// over a color, a round pie, in a layer, repeated under a transparent border, and rays from a
// corner of a wide box.
export default function Scene() {
  const tile = (background: string, style: object = {}) => (
    <View style={{ width: 52, height: 36, background, ...style }} />
  );
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {tile("conic-gradient(#e53935, #fdd835, #43a047, #1e88e5, #e53935)")}
      {tile("conic-gradient(from 45deg, #3949ab, #e8eaf6)")}
      {tile(
        "conic-gradient(at 25% 75%, #00897b 0 25%, #b2dfdb 25% 50%, #00897b 50% 75%, #b2dfdb 75%)",
      )}
      {tile("repeating-conic-gradient(#5d4037 0 15deg, #d7ccc8 15deg 30deg)")}
      {tile("conic-gradient(in oklch longer hue, #e53935, #e53935)")}
      {tile("conic-gradient(from -90deg at 50% 100%, #ff7043, 25%, #6d4c41 50%)")}
      {tile("conic-gradient(#8e24aa 0 70%, #e1bee7 0)")}
      {tile(
        "conic-gradient(from 270deg at right bottom, rgb(0 0 0 / 70%), transparent 90deg), #ffcc80",
      )}
      {tile("conic-gradient(#26a69a 0 0.3turn, #ef5350 0 0.55turn, #ffca28 0)", {
        width: 36,
        borderRadius: "50%",
      })}
      {tile("conic-gradient(from 30deg, #ffeb3b, #f44336, #ffeb3b)", { opacity: 0.5 })}
      {tile("conic-gradient(#ff7043, #6d4c41, #ff7043)", {
        border: "6px solid transparent",
        borderRadius: 10,
      })}
      {tile("repeating-conic-gradient(from 0deg at 0 0, #1e88e5 0 5deg, #bbdefb 5deg 10deg)", {
        width: 112,
      })}
    </View>
  );
}
