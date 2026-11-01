export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// mask-*: a fade over a rounded box, whose shadow and outline the border box cuts; dots sized,
// placed and repeated on one axis; a conic gradient shown by its luminance; two layers, one by
// alpha and one by luminance; a box with a child past its rounded corner, which the rounded
// border box cuts; a masked box with a backdrop filter in it, which sees only what the box
// paints; a backdrop filter faded by its own mask; a mask with a clip path, turned; and a mask
// at an opacity over a filter.
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
          borderRadius: 14,
          boxShadow: "0 0 0 6px #3949ab",
          outline: "3px solid #263238",
          maskImage: "linear-gradient(160deg, #000 30%, transparent 85%)",
        }),
      )}
      {stage(
        box({
          background: "#3949ab",
          maskImage: "radial-gradient(circle, #000 45%, transparent 50%)",
          maskSize: "14px 14px",
          maskPosition: "4px center",
          maskRepeat: "repeat-x",
        }),
      )}
      {stage(
        box({
          background: "#e57373",
          maskImage: "conic-gradient(#fff, #000, #fff)",
          maskMode: "luminance",
        }),
      )}
      {stage(
        box({
          background: "#00897b",
          maskImage:
            "linear-gradient(to right, #000, transparent), radial-gradient(circle, #fff 30%, #000 70%)",
          maskMode: "alpha, luminance",
        }),
      )}
      {stage(
        box(
          {
            borderRadius: 20,
            background: "#ffca28",
            maskImage: "linear-gradient(#000, #000)",
          },
          <View
            style={{
              position: "absolute",
              left: 6,
              top: 6,
              width: 34,
              height: 34,
              background: "#3949ab",
            }}
          />,
        ),
      )}
      {stage(
        box(
          {
            background: "linear-gradient(to right, #3949ab 50%, transparent 50%)",
            maskImage: "linear-gradient(#000, #000)",
          },
          <View style={{ margin: 8, height: 40, backdropFilter: "invert(1)" }} />,
        ),
        { background: warm },
      )}
      {stage(
        box({
          background: "transparent",
          backdropFilter: "invert(1)",
          maskImage: "linear-gradient(to right, #000, transparent)",
        }),
        { background: warm },
      )}
      {stage(
        box({
          transform: "rotate(-20deg)",
          clipPath: "inset(4px round 8px)",
          maskImage: "repeating-linear-gradient(#000 0 6px, transparent 6px 10px)",
        }),
      )}
      {stage(
        box({
          background: "#3949ab",
          opacity: 0.7,
          filter: "drop-shadow(4px 4px 0 #e57373)",
          maskImage: "radial-gradient(#000 40%, transparent 70%)",
        }),
        { background: warm },
      )}
    </View>
  );
}
