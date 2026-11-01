export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// <Mask>: a gradient shown through a letter, and the letter cut out of it; a fade shown by
// luminance; a spot placed absolutely in the source, over a child; stripes in a turned, rounded
// mask; a mask in another's source; luminance inverted; a mask at an opacity over a filter; and a
// mask in another's children.
export default function Scene() {
  const warm = "linear-gradient(135deg, #ffca28, #e57373 50%, #00897b)";
  const stage = (children: any, style?: object) => (
    <View
      style={{
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
  const box = { width: 56, height: 56, background: warm };
  const fill = (style: object) => <View style={{ height: "100%", ...style }} />;
  const letter = (
    <View
      style={{ height: "100%", display: "flex", alignItems: "center", justifyContent: "center" }}
    >
      <Text style={{ fontSize: 52, fontWeight: 900 }}>V</Text>
    </View>
  );
  return (
    <View style={{ padding: 4, display: "flex", flexWrap: "wrap", gap: 6 }}>
      {stage(<Mask source={letter} style={box} />)}
      {stage(<Mask source={letter} invert style={box} />)}
      {stage(
        <Mask
          source={fill({ background: "linear-gradient(to right, #fff, #000)" })}
          mode="luminance"
          style={{ ...box, background: "#3949ab" }}
        />,
      )}
      {stage(
        <Mask
          source={
            <View
              style={{
                position: "absolute",
                left: 18,
                top: 6,
                width: 32,
                height: 32,
                borderRadius: 16,
                background: "#000",
              }}
            />
          }
          style={box}
        >
          <View style={{ margin: 8, height: 20, background: "#3949ab" }} />
        </Mask>,
      )}
      {stage(
        <Mask
          source={fill({
            background: "repeating-linear-gradient(#000 0 6px, transparent 6px 10px)",
          })}
          style={{ ...box, transform: "rotate(-20deg)", borderRadius: 12 }}
        />,
      )}
      {stage(
        <Mask
          source={
            <Mask
              source={letter}
              invert
              style={{
                height: "100%",
                background: "radial-gradient(circle, #000 60%, transparent 62%)",
              }}
            />
          }
          style={box}
        />,
      )}
      {stage(
        <Mask
          source={fill({ background: "radial-gradient(circle, #fff 30%, #000 70%)" })}
          mode="luminance"
          invert
          style={{ ...box, background: "#00897b" }}
        />,
      )}
      {stage(
        <Mask
          source={fill({ borderRadius: 28, background: "#000" })}
          style={{
            ...box,
            background: "transparent",
            opacity: 0.7,
            filter: "drop-shadow(8px 8px 0 #263238)",
          }}
        >
          <View style={{ margin: 6, height: 36, background: "#3949ab" }} />
        </Mask>,
        { background: warm },
      )}
      {stage(
        <Mask source={fill({ background: "linear-gradient(#000, transparent)" })} style={box}>
          <Mask source={letter} style={{ height: "100%", background: "#263238" }} />
        </Mask>,
      )}
    </View>
  );
}
