export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// clip-path: a rounded inset of a box with a border and a shadow; a circle around boxes that
// stick out of it, an absolute and a fixed one; a star filled even-odd, turned; a path of arcs
// and cubic curves; a circle of a backdrop filter; a clipped box with a backdrop filter in it,
// which sees only what the box paints; the drop shadow of a filter cut by an inset past the box;
// a padding box with its corners rounded; and an ellipse at an opacity that blends.
export default function Scene() {
  const warm = "linear-gradient(135deg, #ffca28, #e57373 50%, #00897b)";
  const stage = (children: any, style?: object) => (
    <View
      style={{
        position: "relative",
        width: 76,
        height: 76,
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
  const box = (style: object, children?: any) => (
    <View style={{ width: 56, height: 56, background: warm, ...style }}>{children}</View>
  );
  return (
    <View style={{ padding: 4, display: "flex", flexWrap: "wrap", gap: 6 }}>
      {stage(
        box({
          border: "4px solid #3949ab",
          boxShadow: "0 0 0 6px #00897b",
          clipPath: "inset(-3px 4px 8px round 12px 4px)",
        }),
      )}
      {stage(
        box(
          { clipPath: "circle(30px at 40% 50%)" },
          <>
            <View
              style={{
                position: "absolute",
                left: 40,
                top: 4,
                width: 30,
                height: 30,
                background: "#3949ab",
              }}
            />
            <View
              style={{
                position: "fixed",
                left: 90,
                top: 50,
                width: 60,
                height: 20,
                background: "#8e24aa",
              }}
            />
          </>,
        ),
      )}
      {stage(
        box({
          background: "#3949ab",
          transform: "rotate(15deg)",
          clipPath: "polygon(evenodd, 50% 0, 79% 90%, 2% 35%, 98% 35%, 21% 90%)",
        }),
      )}
      {stage(
        box({
          clipPath:
            "path('M28 50 C10 38 2 28 6 16 A11 11 0 0 1 28 12 A11 11 0 0 1 50 16 C54 28 46 38 28 50 Z')",
        }),
      )}
      {stage(
        box({
          background: "transparent",
          backdropFilter: "invert(1) blur(2px)",
          clipPath: "circle(40%)",
        }),
        { background: warm },
      )}
      {stage(
        box(
          {
            background: "linear-gradient(to right, #3949ab 50%, transparent 50%)",
            clipPath: "inset(0)",
          },
          <View style={{ margin: 8, height: 40, backdropFilter: "invert(1)" }} />,
        ),
        { background: warm },
      )}
      {stage(
        box({
          width: 40,
          height: 40,
          background: "#ffca28",
          borderRadius: 8,
          filter: "drop-shadow(6px 6px 2px #3949ab)",
          clipPath: "inset(-4px)",
        }),
      )}
      {stage(
        box(
          {
            border: "8px solid #e57373",
            padding: 4,
            borderRadius: 20,
            clipPath: "padding-box",
          },
          <View style={{ width: 60, height: 60, background: "#00897b80" }} />,
        ),
      )}
      {stage(
        box({
          background: "#90caf9",
          opacity: 0.8,
          mixBlendMode: "difference",
          clipPath: "ellipse(45% 30% at 50% 50%)",
        }),
        { background: warm },
      )}
    </View>
  );
}
