export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// transform-style: preserve-3d: a cube with its faces' backs hidden, two cards that cross, drawn
// in parts, opaque and translucent, a translucent box nearer the viewer drawn over one after it,
// a card flipped with its faces' backs hidden, turns that add up through two levels, and an
// opacity that flattens what is in it.
export default function Scene() {
  const stage = (children: any, style?: object) => (
    <View
      style={{
        width: 72,
        height: 72,
        perspective: 300,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: "#eceff1",
        ...style,
      }}
    >
      {children}
    </View>
  );
  const face = (background: string, transform: string) => (
    <View
      style={{
        position: "absolute",
        left: 0,
        top: 0,
        width: 36,
        height: 36,
        background,
        transform,
        backfaceVisibility: "hidden",
      }}
    />
  );
  const cube = (
    <View
      style={{
        position: "relative",
        width: 36,
        height: 36,
        transformStyle: "preserve-3d",
        transform: "rotateX(-25deg) rotateY(35deg)",
      }}
    >
      {face("#3949ab", "translateZ(18px)")}
      {face("#e57373", "rotateY(90deg) translateZ(18px)")}
      {face("#ffca28", "rotateY(180deg) translateZ(18px)")}
      {face("#00897b", "rotateY(-90deg) translateZ(18px)")}
      {face("#8e24aa", "rotateX(90deg) translateZ(18px)")}
      {face("#ef6c00", "rotateX(-90deg) translateZ(18px)")}
    </View>
  );
  const pair = (style: object, a: object, b: object, c?: object) => (
    <View
      style={{
        position: "relative",
        width: 48,
        height: 36,
        transformStyle: "preserve-3d",
        ...style,
      }}
    >
      <View style={{ position: "absolute", inset: 0, background: "#3949ab", ...a }} />
      <View style={{ position: "absolute", inset: 0, background: "#e57373", ...b }} />
      {c}
    </View>
  );
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {stage(cube)}
      {stage(pair({}, { transform: "rotateY(50deg)" }, { transform: "rotateY(-50deg)" }))}
      {stage(
        pair(
          {},
          { transform: "rotateY(50deg)", background: "rgb(57 73 171 / 0.6)" },
          { transform: "rotateY(-50deg)", background: "rgb(229 115 115 / 0.6)" },
        ),
      )}
      {stage(
        pair(
          {},
          { transform: "translateZ(20px)", left: 16, top: 12, background: "rgb(57 73 171 / 0.7)" },
          { transform: "translateZ(-10px)", right: 16, bottom: 12 },
        ),
      )}
      {stage(
        pair(
          { transform: "rotateY(150deg)" },
          { backfaceVisibility: "hidden" },
          { transform: "rotateY(180deg)", backfaceVisibility: "hidden", background: "#ffca28" },
        ),
      )}
      {stage(
        pair(
          { transform: "rotateY(30deg)" },
          { transform: "translateZ(-20px)", opacity: 0.5 },
          {
            transformStyle: "preserve-3d",
            transform: "rotateY(30deg)",
            background: "#c8e6c9",
          },
          <View
            style={{
              position: "fixed",
              left: 4,
              top: 4,
              width: 12,
              height: 12,
              background: "#2e7d32",
              transform: "rotateX(60deg)",
            }}
          />,
        ),
      )}
      {stage(
        pair(
          { opacity: 0.8 },
          { transform: "translateZ(20px)", left: 16, top: 12 },
          { transform: "translateZ(-10px)", right: 16, bottom: 12 },
        ),
      )}
    </View>
  );
}
