export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// backdrop-filter: frosted glass over stripes; a blur across an edge between colors, which reads
// only what is in the box; through the rounded clip around it, and through a turned box in a
// clip; under a box with an opacity, which is all it sees; colors; with an opacity of its own;
// turned; and a box in a frosted box, which sees that box.
export default function Scene() {
  const stripes = "repeating-linear-gradient(45deg, #3949ab 0 6px, #ffca28 6px 12px)";
  const stage = (children: any, style?: object) => (
    <View
      style={{
        position: "relative",
        width: 76,
        height: 76,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: stripes,
        ...style,
      }}
    >
      {children}
    </View>
  );
  const glass = (style: object, children?: any) => (
    <View style={{ width: 44, height: 44, backdropFilter: "blur(4px)", ...style }}>{children}</View>
  );
  return (
    <View style={{ padding: 4, display: "flex", flexWrap: "wrap", gap: 6 }}>
      {stage(glass({ borderRadius: 12, background: "#ffffff40", border: "1px solid #fff" }))}
      {stage(glass({ marginLeft: 20, backdropFilter: "blur(6px)" }), {
        background: "linear-gradient(to right, #e53935 50%, #1e88e5 50%)",
      })}
      {stage(
        <View
          style={{
            width: 60,
            height: 60,
            borderRadius: 18,
            overflow: "hidden",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            background: "radial-gradient(circle, #00897b 0 30%, #fff 31%)",
          }}
        >
          {glass({ width: 70, height: 24 })}
        </View>,
      )}
      {stage(
        <View style={{ width: 64, height: 64, overflow: "hidden", display: "flex" }}>
          <View
            style={{
              margin: 8,
              flexGrow: 1,
              transform: "rotate(20deg)",
              background: "conic-gradient(#e53935, #fdd835, #43a047, #1e88e5, #e53935)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            {glass({ width: 30, height: 30 })}
          </View>
        </View>,
      )}
      {stage(
        <View style={{ width: 60, height: 60, opacity: 0.9 }}>
          <View
            style={{
              position: "absolute",
              left: 6,
              top: 6,
              width: 20,
              height: 20,
              background: "#e53935",
            }}
          />
          {glass({ position: "absolute", left: 10, top: 10, width: 40, height: 40 })}
        </View>,
      )}
      {stage(
        <>
          {glass({
            position: "absolute",
            left: 8,
            top: 8,
            width: 30,
            height: 60,
            backdropFilter: "grayscale(1)",
          })}
          {glass({
            position: "absolute",
            left: 40,
            top: 8,
            width: 28,
            height: 60,
            backdropFilter: "hue-rotate(120deg) contrast(1.5)",
          })}
        </>,
        { background: "conic-gradient(#e53935, #fdd835, #43a047, #1e88e5, #8e24aa, #e53935)" },
      )}
      {stage(glass({ backdropFilter: "invert(1)", opacity: 0.5, borderRadius: "50%" }))}
      {stage(
        glass({ transform: "rotate(15deg)", backdropFilter: "blur(3px)", background: "#ffffff30" }),
      )}
      {stage(
        glass(
          {
            width: 60,
            height: 60,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            background: "#ffffff30",
          },
          glass({ width: 28, height: 28, backdropFilter: "grayscale(1) brightness(1.4)" }),
        ),
      )}
    </View>
  );
}
