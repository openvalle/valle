export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// backface-visibility: a box turned to its back, shown and hidden; a card flipped in perspective,
// its front hidden and its back shown; a box turned past its edge but seen from the side, which
// shows its front; one turned back in a parent turned around, which does not count; a mirror;
// and a box turned over with a child, both hidden.
export default function Scene() {
  const stage = (style: object, children: any) => (
    <View style={{ width: 116, height: 52, background: "#eceff1", ...style }}>{children}</View>
  );
  const face = (style: object) => (
    <View
      style={{
        width: 44,
        height: 32,
        background: "linear-gradient(to right, #3949ab, #e57373)",
        backfaceVisibility: "hidden",
        ...style,
      }}
    />
  );
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {stage(
        { display: "flex", justifyContent: "space-around", alignItems: "center" },
        <>
          {face({ transform: "rotateY(180deg)", backfaceVisibility: "visible" })}
          {face({ transform: "rotateY(180deg)" })}
        </>,
      )}
      {stage(
        { perspective: 200, position: "relative" },
        <>
          {face({
            position: "absolute",
            left: 30,
            top: 10,
            width: 56,
            transform: "rotateY(140deg)",
          })}
          {face({
            position: "absolute",
            left: 30,
            top: 10,
            width: 56,
            background: "#ffca28",
            transform: "rotateY(-40deg)",
          })}
        </>,
      )}
      {stage(
        { perspective: 60, perspectiveOrigin: "100% 50%", padding: 10 },
        face({ width: 24, transform: "rotateY(100deg)" }),
      )}
      {stage(
        { padding: 10 },
        <View style={{ width: 44, transform: "rotateY(180deg)" }}>
          {face({ transform: "rotateY(0deg)" })}
        </View>,
      )}
      {stage({ padding: 10 }, face({ transform: "scaleX(-1)" }))}
      {stage(
        { padding: 10 },
        <View
          style={{
            width: 44,
            height: 32,
            background: "#8e24aa",
            backfaceVisibility: "hidden",
            transform: "rotateX(180deg)",
          }}
        >
          <View style={{ width: 20, height: 10, background: "#2e7d32" }} />
        </View>,
      )}
    </View>
  );
}
