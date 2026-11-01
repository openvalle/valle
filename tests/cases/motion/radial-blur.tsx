export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// radial-blur(): dots streaked out from the box's center, sharp near it; text from a point off
// its middle; bars from their own top left, turned and scaled with them; and what is behind a
// box, a little and a lot.
export default function Scene() {
  const stage = (children: any, style?: object) => (
    <View
      style={{
        width: 116,
        height: 116,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: "#1a237e",
        ...style,
      }}
    >
      {children}
    </View>
  );
  const dots = "radial-gradient(circle, #fff59d 3px, transparent 3.5px) 0 0 / 14px 14px";
  const bars = "repeating-linear-gradient(90deg, #80deea 0 6px, transparent 6px 14px)";
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 4 }}>
      {stage(
        <View
          style={{
            width: 100,
            height: 100,
            background: dots,
            filter: "radial-blur(50px 50px 16px)",
          }}
        />,
      )}
      {stage(
        <Text
          style={{
            fontSize: 40,
            fontWeight: 900,
            color: "white",
            filter: "radial-blur(30px 10px 6px)",
          }}
        >
          Valle
        </Text>,
      )}
      {stage(
        <View
          style={{
            width: 72,
            height: 40,
            background: bars,
            rotate: "30deg",
            scale: 1.25,
            filter: "radial-blur(0 0 10px)",
          }}
        />,
      )}
      {stage(
        <>
          <View
            style={{
              width: 88,
              height: 36,
              borderRadius: 18,
              backdropFilter: "radial-blur(44px 18px 1px)",
            }}
          />
          <View
            style={{
              width: 88,
              height: 36,
              borderRadius: 18,
              backdropFilter: "radial-blur(44px 18px 20px)",
              background: "rgb(255 255 255 / 10%)",
            }}
          />
        </>,
        { background: dots + ", #37474f", flexDirection: "column", gap: 12 },
      )}
    </View>
  );
}
