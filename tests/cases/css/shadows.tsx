export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Box shadows: a blurred card shadow with rounded corners, none under a translucent background,
// spread with a small radius, an elliptical shadow, inner shadows inside a border, rings from
// spread, several shadows in order, a shadow in a layer, and one that runs off the canvas.
export default function Scene() {
  const box = (style: object) => (
    <View style={{ width: 56, height: 36, background: "#ffffff", ...style }} />
  );
  return (
    <View
      style={{
        height: 256,
        padding: 14,
        display: "flex",
        flexWrap: "wrap",
        gap: 20,
        alignContent: "flex-start",
        background: "#eceff1",
      }}
    >
      {box({ borderRadius: 10, boxShadow: "0 6px 12px rgb(0 0 0 / 35%)" })}
      {box({ background: "rgb(255 255 255 / 40%)", boxShadow: "6px 6px 0 #5c6bc0" })}
      {box({ borderRadius: 3, boxShadow: "0 0 0 8px #26a69a" })}
      {box({ borderRadius: "50%", boxShadow: "4px 4px 8px 2px #ef5350" })}
      {box({
        border: "4px solid #8d6e63",
        borderRadius: 14,
        boxShadow: "inset 4px 6px 8px rgb(0 0 0 / 50%)",
      })}
      {box({ borderRadius: 8, boxShadow: "inset 0 0 0 6px #ab47bc, inset 0 0 6px 10px #e1bee7" })}
      {box({
        borderRadius: 6,
        boxShadow: "0 0 0 3px #ffca28, 0 0 0 6px #fb8c00, 10px 10px 10px #6d4c41",
      })}
      {box({ opacity: 0.5, boxShadow: "-6px -6px 4px #1e88e5" })}
      {box({ boxShadow: "0 0 0 -30px red" })}
      {box({ position: "absolute", left: 226, top: 200, boxShadow: "0 0 16px 4px #43a047" })}
    </View>
  );
}
