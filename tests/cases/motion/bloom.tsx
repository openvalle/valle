export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// bloom(): bright text glowing past itself; on a whole box, only what is past the threshold,
// eased in over the knee; a hard threshold on stripes, turned and scaled with them; and what is
// behind a box.
export default function Scene() {
  const stage = (children: any, style?: object) => (
    <View
      style={{
        width: 116,
        height: 116,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: "#1a1a2e",
        ...style,
      }}
    >
      {children}
    </View>
  );
  const dots = "radial-gradient(circle, #ffffff 3px, transparent 3.5px) 0 0 / 20px 20px";
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 4 }}>
      {stage(
        <Text
          style={{
            fontSize: 40,
            fontWeight: 900,
            color: "#80deea",
            filter: "bloom(12px 1.5 0.6)",
          }}
        >
          Valle
        </Text>,
      )}
      {stage(null, {
        background:
          "radial-gradient(circle, #ffffff 6px, transparent 6.5px) 0 0 / 28px 28px, " +
          "linear-gradient(90deg, #311b92, #ff8a65 50%, #ffeb3b)",
        filter: "bloom(8px 2 0.75 0.2)",
      })}
      {stage(
        <View
          style={{
            width: 64,
            height: 40,
            background: "repeating-linear-gradient(90deg, #f48fb1 0 4px, #4a148c 4px 12px)",
            rotate: "-25deg",
            scale: 1.25,
            filter: "bloom(8px 2 0.5 0)",
          }}
        />,
      )}
      {stage(
        <View
          style={{
            width: 88,
            height: 48,
            borderRadius: 24,
            backdropFilter: "bloom(10px 1 0.7)",
            border: "1px solid rgb(255 255 255 / 30%)",
          }}
        />,
        { background: dots + ", #263238" },
      )}
    </View>
  );
}
