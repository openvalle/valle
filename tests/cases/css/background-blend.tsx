export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// background-blend-mode: an image that blends only with the box's own background, not with what
// is under the box; three images and two modes, repeated, in a rounded border; hue and
// luminosity of a conic gradient over a linear one; and a translucent color under an image that
// multiplies, over the page.
export default function Scene() {
  const warm = "linear-gradient(135deg, #ffca28, #e57373 50%, #00897b)";
  const stage = (children: any, style?: object) => (
    <View
      style={{
        width: 76,
        height: 76,
        display: "flex",
        flexDirection: "column",
        gap: 6,
        padding: 6,
        background: warm,
        ...style,
      }}
    >
      {children}
    </View>
  );
  const stripes = "repeating-linear-gradient(45deg, #3949ab 0 6px, #90caf9 6px 12px)";
  return (
    <View style={{ padding: 4, display: "flex", flexWrap: "wrap", gap: 6 }}>
      {stage(
        <>
          <View style={{ flexGrow: 1, background: stripes, backgroundBlendMode: "difference" }} />
          <View style={{ flexGrow: 1, background: stripes, mixBlendMode: "difference" }} />
        </>,
      )}
      {stage(
        <View
          style={{
            flexGrow: 1,
            border: "4px solid #263238",
            borderRadius: 12,
            backgroundImage: `radial-gradient(circle, #fff, #fff0 70%), ${stripes}, linear-gradient(#e57373, #ffca28)`,
            backgroundSize: "24px 24px, auto, auto",
            backgroundColor: "#00897b",
            backgroundBlendMode: "overlay, multiply",
          }}
        />,
        { background: "#eceff1" },
      )}
      {stage(
        ["hue", "luminosity"].map((mode) => (
          <View
            style={{
              flexGrow: 1,
              background:
                "conic-gradient(#e53935, #fdd835, #43a047, #1e88e5, #8e24aa, #e53935), linear-gradient(to right, #00acc1, #fff, #6d4c41)",
              backgroundBlendMode: mode,
            }}
          />
        )),
      )}
      {stage(
        <View
          style={{
            flexGrow: 1,
            margin: 6,
            background: `${stripes}, #ffffff80`,
            backgroundBlendMode: "multiply",
          }}
        />,
      )}
    </View>
  );
}
