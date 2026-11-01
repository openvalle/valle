export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// 3D transforms, each flattened onto its parent's plane: turned about y and x in perspective with
// a border and rounded corners, moved toward the viewer, turned about a diagonal without
// perspective, a matrix3d, an origin behind the box, a turned box in a turned box, the
// individual properties' 3D forms and text tipped back; behind them, a box turned so far that
// part of it is behind the viewer.
export default function Scene() {
  const tile = (style: object, children?: any) => (
    <View style={{ width: 52, height: 36, background: "#3949ab", ...style }}>{children}</View>
  );
  return (
    <View style={{ padding: 6, display: "flex", flexWrap: "wrap", gap: 12 }}>
      {tile({
        position: "absolute",
        right: 6,
        bottom: 6,
        width: 80,
        transform: "perspective(36px) rotateY(-80deg)",
        background: "#00acc1",
      })}
      {tile({
        transform: "perspective(120px) rotateY(50deg)",
        border: "4px solid #e57373",
        borderRadius: 10,
        background: "#c5cae9",
      })}
      {tile({
        transform: "perspective(100px) rotateX(-45deg)",
        background: "linear-gradient(135deg, #00897b, #b2dfdb)",
      })}
      {tile({ transform: "perspective(200px) translateZ(40px)", background: "#ffca28" })}
      {tile({ transform: "rotate3d(1, 1, 0, 60deg)", background: "#8e24aa" })}
      {tile({
        transform: "matrix3d(1, 0, 0, 0.004, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)",
        background: "#b3e5fc",
      })}
      {tile({
        transformOrigin: "center center -40px",
        transform: "perspective(150px) rotateY(-35deg)",
        background: "#ef6c00",
      })}
      {tile(
        { transform: "perspective(150px) rotateY(40deg)", background: "#c8e6c9" },
        <View
          style={{
            margin: 8,
            height: 20,
            transform: "perspective(80px) rotateX(50deg)",
            background: "#2e7d32",
          }}
        />,
      )}
      {tile({
        rotate: "1 1 0 50deg",
        scale: "1 1 2",
        translate: "4px 0 30px",
        background: "#f06292",
      })}
      {tile(
        {
          transform: "perspective(120px) rotateX(35deg)",
          background: "#fff3e0",
          color: "#ef6c00",
          fontSize: 16,
        },
        <Text>Valle</Text>,
      )}
    </View>
  );
}
