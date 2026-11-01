export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Perspective: cards turned either way about y on each side of the origin, an origin at the top
// left, boxes moved away and toward the viewer, a grandchild it does not reach, and absolute and
// fixed boxes in it, which it contains.
export default function Scene() {
  const stage = (style: object, children: any) => (
    <View style={{ width: 116, height: 60, background: "#eceff1", ...style }}>{children}</View>
  );
  const card = (background: string, style: object) => (
    <View style={{ width: 44, height: 36, background, ...style }} />
  );
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {stage(
        { perspective: 150, display: "flex", justifyContent: "space-between", padding: 8 },
        <>
          {card("#3949ab", { transform: "rotateY(45deg)" })}
          {card("#e57373", { transform: "rotateY(-45deg)" })}
        </>,
      )}
      {stage(
        { perspective: "100px", perspectiveOrigin: "left top", padding: 12 },
        card("linear-gradient(135deg, #00897b, #b2dfdb)", {
          width: 80,
          transform: "rotateX(40deg)",
          borderRadius: 8,
        }),
      )}
      {stage(
        { perspective: 200, position: "relative" },
        <>
          {card("#8e24aa", {
            position: "absolute",
            left: 8,
            top: 12,
            transform: "translateZ(-60px)",
          })}
          {card("#ffca28", {
            position: "absolute",
            left: 50,
            top: 12,
            transform: "translateZ(40px)",
          })}
        </>,
      )}
      {stage(
        { perspective: 120, padding: 12 },
        <View style={{ height: 36 }}>
          {card("#ef6c00", { width: 80, transform: "rotateY(60deg)" })}
        </View>,
      )}
      {stage(
        { perspective: 150 },
        <>
          {card("#2e7d32", {
            position: "absolute",
            right: 4,
            bottom: 4,
            height: 24,
            transform: "rotateY(50deg)",
          })}
          {card("#b3e5fc", {
            position: "fixed",
            left: 4,
            top: 4,
            height: 24,
            transform: "rotateX(-50deg)",
          })}
        </>,
      )}
    </View>
  );
}
