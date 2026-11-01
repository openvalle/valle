export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// 2D transforms: a turned box with a border and rounded corners, a scaled gradient, skewed text,
// a matrix, a moved and turned box with a shadow, a turn undone inside another, a clip turned
// with its box, a turned layer, a turn about a corner, a mirrored conic gradient, a turned box
// over the boxes after it, and turned text.
export default function Scene() {
  const tile = (style: object, children?: any) => (
    <View style={{ width: 52, height: 36, background: "#3949ab", ...style }}>{children}</View>
  );
  return (
    <View style={{ padding: 6, display: "flex", flexWrap: "wrap", gap: 12 }}>
      {tile({
        transform: "rotate(15deg)",
        border: "4px solid #e57373",
        borderRadius: 10,
        background: "#c5cae9",
      })}
      {tile({
        transform: "scale(0.7)",
        background: "linear-gradient(135deg, #00897b, #b2dfdb)",
      })}
      {tile(
        { transform: "skewX(-15deg)", background: "#fff3e0", color: "#ef6c00", fontSize: 14 },
        <Text>Valle</Text>,
      )}
      {tile({ transform: "matrix(1, 0.2, -0.3, 0.9, 4, -2)", background: "#8e24aa" })}
      {tile({
        transform: "translate(6px, 4px) rotate(-20deg)",
        boxShadow: "4px 6px 8px rgb(0 0 0 / 40%)",
        background: "#ffca28",
      })}
      {tile(
        { transform: "rotate(25deg)", background: "#b3e5fc" },
        <View
          style={{
            margin: 8,
            height: 20,
            transform: "rotate(-25deg) scale(0.8)",
            background: "#0277bd",
          }}
        />,
      )}
      {tile(
        { transform: "rotate(-12deg)", overflow: "hidden", borderRadius: 8, background: "#f8bbd0" },
        <View style={{ width: 70, height: 20, marginLeft: -10, background: "#c2185b" }} />,
      )}
      {tile({ transform: "rotate(45deg) scale(0.6)", opacity: 0.6, background: "#43a047" })}
      {tile({ transform: "rotate(10deg)", transformOrigin: "left top", background: "#6d4c41" })}
      {tile({
        transform: "scale(-1, 1)",
        background: "conic-gradient(from 30deg, #ffeb3b, #f44336, #3f51b5, #ffeb3b)",
      })}
      {tile({ transform: "rotate(-8deg) translateX(20px)", background: "#26a69a" })}
      {tile({ background: "#ffccbc" })}
      {tile(
        { transform: "rotate(90deg)", background: "#e8eaf6", color: "#283593", fontSize: 12 },
        <Text>Turned</Text>,
      )}
    </View>
  );
}
