export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Linear gradients: down by default, to a side, at an angle and to a corner of a wide box, with a
// hint, hard stops and two positions, repeating stripes, mixed in Oklab, Oklch and linear sRGB,
// fading to transparent, layers over a color, repeated under a transparent border, inside a
// rounded opaque border, in a layer, and clipped by an ancestor.
export default function Scene() {
  const tile = (background: string, style: object = {}) => (
    <View style={{ width: 52, height: 36, background, ...style }} />
  );
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {tile("linear-gradient(#1e88e5, #e53935)")}
      {tile("linear-gradient(to right, #fdd835, #43a047, #1e88e5)")}
      {tile("linear-gradient(30deg, #8e24aa, #fb8c00)", { width: 112 })}
      {tile("linear-gradient(to top left, #00897b, #fff59d 40%, #d81b60)", { width: 112 })}
      {tile("linear-gradient(to right, #3949ab, 20%, #e8eaf6)")}
      {tile("linear-gradient(to right, #c62828 0 30%, #fbc02d 30% 60%, #2e7d32 60%)")}
      {tile("repeating-linear-gradient(-45deg, #5d4037 0 5px, #d7ccc8 5px 10px)")}
      {tile("linear-gradient(to right in oklab, #e53935, #1e88e5)")}
      {tile("linear-gradient(to right in oklch, #e53935, #1e88e5)")}
      {tile("linear-gradient(to right in srgb-linear, #e53935, #1e88e5)")}
      {tile("linear-gradient(to right, rgb(0 0 0 / 80%), transparent), #ffccbc")}
      {tile(
        "linear-gradient(to bottom, transparent 50%, rgb(255 255 255 / 70%)), linear-gradient(to right, #7b1fa2, #0288d1)",
      )}
      {tile("linear-gradient(#ff7043, #6d4c41)", {
        border: "8px solid transparent",
        borderRadius: 10,
      })}
      {tile("linear-gradient(to right, #26c6da, #ab47bc)", {
        border: "4px solid #37474f",
        borderRadius: 14,
      })}
      {tile("linear-gradient(45deg, #ffeb3b, #f44336)", { opacity: 0.5 })}
      <View style={{ overflow: "hidden", width: 52, height: 36, borderRadius: 18 }}>
        {tile("linear-gradient(to bottom right, #1b5e20, #a5d6a7)", { width: 80, height: 50 })}
      </View>
    </View>
  );
}
