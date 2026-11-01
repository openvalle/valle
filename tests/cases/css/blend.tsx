export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// mix-blend-mode and isolation: a box blending through the rounded clip of a box around it, which
// makes no layer; the same blend in a plain box and in an isolated one, which keeps it from what
// is under it; a transformed box and a box with a z-index, which isolate too; an opacity on a
// box that blends over a translucent one and over nothing; hue, saturation, color and
// luminosity; plus-lighter; a blend in a box that blends; and isolation flattening a 3D box.
export default function Scene() {
  const warm = "linear-gradient(135deg, #ffca28, #e57373 50%, #00897b)";
  const stage = (children: any, style?: object) => (
    <View
      style={{
        position: "relative",
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
  const chip = (style: object) => (
    <View style={{ flexGrow: 1, background: "#90caf9", mixBlendMode: "difference", ...style }} />
  );
  const card = (background: string, transform: string) => (
    <View
      style={{
        position: "absolute",
        left: 10,
        top: 10,
        width: 28,
        height: 28,
        background,
        transform,
      }}
    />
  );
  return (
    <View style={{ padding: 4, display: "flex", flexWrap: "wrap", gap: 6 }}>
      {stage(
        <View
          style={{ flexGrow: 1, display: "flex", margin: 4, borderRadius: 14, overflow: "hidden" }}
        >
          <View
            style={{
              margin: -8,
              flexGrow: 1,
              mixBlendMode: "multiply",
              background: "conic-gradient(#3949ab, #00acc1, #fff, #3949ab)",
            }}
          />
        </View>,
      )}
      {stage(
        <>
          <View style={{ flexGrow: 1, display: "flex" }}>{chip({})}</View>
          <View style={{ flexGrow: 1, display: "flex", isolation: "isolate" }}>{chip({})}</View>
        </>,
      )}
      {stage(
        <>
          <View
            style={{
              flexGrow: 1,
              display: "flex",
              padding: 4,
              background: "#ffffff80",
              transform: "rotate(8deg)",
            }}
          >
            {chip({ mixBlendMode: "screen" })}
          </View>
          <View
            style={{
              flexGrow: 1,
              display: "flex",
              padding: 4,
              position: "relative",
              zIndex: 0,
            }}
          >
            {chip({ mixBlendMode: "multiply" })}
          </View>
        </>,
      )}
      {stage(
        <>
          <View
            style={{
              position: "absolute",
              left: 8,
              top: 8,
              width: 40,
              height: 40,
              background: "#00897b99",
            }}
          />
          <View
            style={{
              position: "absolute",
              left: 26,
              top: 26,
              width: 44,
              height: 44,
              borderRadius: 10,
              background: "#3949abcc",
              mixBlendMode: "hard-light",
              opacity: 0.7,
            }}
          />
        </>,
        { background: "none" },
      )}
      {stage(
        ["hue", "saturation", "color", "luminosity"].map((mode) => (
          <View
            style={{
              flexGrow: 1,
              mixBlendMode: mode,
              background: "linear-gradient(to right, #00acc1, #ffffff80, #6d4c41)",
            }}
          />
        )),
        {
          gap: 2,
          background: "linear-gradient(#e53935, #fdd835, #43a047, #1e88e5, #8e24aa)",
        },
      )}
      {stage(
        <>
          <View
            style={{
              position: "absolute",
              left: 6,
              top: 6,
              width: 40,
              height: 40,
              background: "#e5737399",
              mixBlendMode: "plus-lighter",
            }}
          />
          <View
            style={{
              position: "absolute",
              left: 30,
              top: 30,
              width: 40,
              height: 40,
              borderRadius: 20,
              background: "#3949ab",
              mixBlendMode: "plus-lighter",
            }}
          />
        </>,
        { background: "linear-gradient(to right, #263238, #263238 50%, transparent 50%)" },
      )}
      {stage(
        <View
          style={{
            flexGrow: 1,
            display: "flex",
            padding: 10,
            background: "#b2dfdb",
            mixBlendMode: "multiply",
          }}
        >
          {chip({ background: "linear-gradient(#3949ab, #fff)" })}
        </View>,
      )}
      {stage(
        <View
          style={{
            flexGrow: 1,
            perspective: 200,
            isolation: "isolate",
            transformStyle: "preserve-3d",
          }}
        >
          {card("#3949ab", "rotateY(50deg)")}
          {card("#e57373", "rotateY(-50deg)")}
        </View>,
        { background: "#eceff1" },
      )}
    </View>
  );
}
