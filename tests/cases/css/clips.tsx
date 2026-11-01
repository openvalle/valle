export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Backgrounds in a box's areas: under a translucent border, in the padding box, in the content
// box with rounded corners less the border and padding, a color in one box and an image placed
// in another, tiles from the border box shown in the content box, and layers in each box.
export default function Scene() {
  const tile = (background: string, style: object = {}) => (
    <View
      style={{
        width: 52,
        height: 36,
        padding: "4px 6px",
        border: "5px solid rgb(0 0 0 / 25%)",
        background,
        ...style,
      }}
    />
  );
  const stripes = "repeating-linear-gradient(45deg, #3949ab 0 4px, #c5cae9 4px 8px)";
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {tile(`${stripes}, #ffcc80`)}
      {tile(`${stripes} padding-box, #ffcc80 padding-box`)}
      {tile(`${stripes} content-box, #ffcc80 content-box`, { borderRadius: 18 })}
      {tile(`linear-gradient(#00897b, #b2dfdb) border-box padding-box, #ffcc80 content-box`)}
      {tile("radial-gradient(#e53935, #ffcdd2) border-box content-box no-repeat", {
        borderRadius: "12px 4px",
      })}
      {tile(
        "linear-gradient(#ffca28, #ef6c00) content-box, " +
          "linear-gradient(#26a69a, #e0f2f1) padding-box, #8e24aa",
        { borderRadius: 10 },
      )}
      {tile(`${stripes}`, { backgroundClip: "content-box", backgroundOrigin: "border-box" })}
      {tile("conic-gradient(#ff7043, #6d4c41, #ff7043) 0 0 / 50% 50%", {
        backgroundOrigin: "content-box",
        backgroundClip: "padding-box",
        borderRadius: "50%",
      })}
    </View>
  );
}
