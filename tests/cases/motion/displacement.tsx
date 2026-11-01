export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// displacement(): text in a heat haze, waving more along y than x; stripes through turbulence;
// a grid through one octave, turned and scaled with it; and what is behind a box, through four.
export default function Scene() {
  const stage = (children: any, style?: object) => (
    <View
      style={{
        width: 116,
        height: 116,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: "#004d40",
        ...style,
      }}
    >
      {children}
    </View>
  );
  const bars = "repeating-linear-gradient(90deg, #ffcc80 0 6px, transparent 6px 14px)";
  const grid =
    "linear-gradient(#ffcc80 2px, transparent 2px) 0 0 / 12px 12px, " +
    "linear-gradient(90deg, #ffcc80 2px, transparent 2px) 0 0 / 12px 12px";
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 4 }}>
      {stage(
        <Text
          style={{
            fontSize: 40,
            fontWeight: 900,
            color: "white",
            filter: "displacement(7 0.02 0.08 8px)",
          }}
        >
          Valle
        </Text>,
      )}
      {stage(
        <View
          style={{
            width: 96,
            height: 96,
            background: bars,
            filter: "displacement(3 0.03 0.03 16px 3 turbulence)",
          }}
        />,
      )}
      {stage(
        <View
          style={{
            width: 72,
            height: 72,
            background: grid,
            rotate: "20deg",
            scale: 1.2,
            filter: "displacement(1 0.06 0.06 10px 1)",
          }}
        />,
      )}
      {stage(
        <View
          style={{
            width: 88,
            height: 48,
            borderRadius: 24,
            backdropFilter: "displacement(9 0.05 0.05 12px 4 fractal)",
            background: "rgb(255 255 255 / 10%)",
          }}
        />,
        { background: bars + ", #37474f" },
      )}
    </View>
  );
}
