export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// filter: a blurred box with a border and rounded corners; a chain of color functions; drop
// shadows of a shape that sticks out, of a translucent one twice, and of a turned one; a blur in
// the rounded clip around it under an opacity; boxes placed in a filtered box, which contains
// them; colors pushed past their range; and a blurred box that blends.
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
    <View style={{ width: 40, height: 40, background: "#3949ab", ...style }}>{children}</View>
  );
  return (
    <View style={{ padding: 4, display: "flex", flexWrap: "wrap", gap: 6 }}>
      {stage(
        box({
          border: "4px solid #e57373",
          borderRadius: 10,
          filter: "blur(3px)",
        }),
      )}
      {stage(
        box({
          width: 56,
          height: 56,
          background: `radial-gradient(circle, #fff 0 20%, transparent 21%), ${warm}`,
          filter: "grayscale(0.5) sepia(0.6) hue-rotate(200deg) saturate(1.8)",
        }),
      )}
      {stage(
        box(
          {
            borderRadius: 20,
            background: "#ffca28",
            filter: "drop-shadow(3px 4px 2px #3949abcc)",
          },
          <View
            style={{
              position: "absolute",
              left: 50,
              top: 10,
              width: 16,
              height: 16,
              background: "#00897b",
            }}
          />,
        ),
        { justifyContent: "flex-start", paddingLeft: 8 },
      )}
      {stage(
        box({
          background: "linear-gradient(#e5737380, #3949ab)",
          borderRadius: "50% 0",
          filter: "drop-shadow(-6px 0 0 #00897b) drop-shadow(0 6px 1px #ef6c00)",
        }),
      )}
      {stage(
        box({
          width: 34,
          height: 20,
          background: "#8e24aa",
          transform: "rotate(30deg) scale(1.3)",
          filter: "drop-shadow(6px 0 1px #263238)",
        }),
      )}
      {stage(
        <View
          style={{ width: 60, height: 60, borderRadius: 16, overflow: "hidden", background: warm }}
        >
          {box({ margin: 10, opacity: 0.6, background: "#fff", filter: "blur(5px)" })}
        </View>,
      )}
      {stage(
        box(
          { filter: "hue-rotate(120deg)", width: 50, height: 50 },
          <>
            <View
              style={{
                position: "absolute",
                right: 4,
                bottom: 4,
                width: 14,
                height: 14,
                background: "#e57373",
              }}
            />
            <View
              style={{
                position: "fixed",
                left: 4,
                top: 4,
                width: 14,
                height: 14,
                background: "#ffca28",
              }}
            />
          </>,
        ),
      )}
      {stage(
        box({
          width: 60,
          height: 60,
          borderRadius: "50%",
          background: "conic-gradient(#e53935, #fdd835, #43a047, #1e88e5, #8e24aa, #e53935)",
          filter: "brightness(1.6) contrast(1.4) saturate(2) invert(0.2)",
        }),
      )}
      {stage(
        box({
          background: "#90caf9",
          filter: "blur(2px)",
          mixBlendMode: "difference",
        }),
        { background: warm },
      )}
    </View>
  );
}
