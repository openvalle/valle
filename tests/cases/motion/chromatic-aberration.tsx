export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// chromatic-aberration(): red on and blue back along the box's x, on text; the other way, on a
// ring; turned and scaled with the box; translucent; and on what is behind a box.
export default function Scene() {
  const stage = (children: any, style?: object) => (
    <View
      style={{
        width: 116,
        height: 116,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: "#263238",
        ...style,
      }}
    >
      {children}
    </View>
  );
  const bars = {
    width: 72,
    height: 40,
    background: "repeating-linear-gradient(90deg, white 0 6px, transparent 6px 14px)",
  };
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 4 }}>
      {stage(
        <Text
          style={{
            fontSize: 40,
            fontWeight: 900,
            color: "white",
            filter: "chromatic-aberration(6px)",
          }}
        >
          Valle
        </Text>,
      )}
      {stage(
        <View
          style={{
            width: 64,
            height: 64,
            borderRadius: 32,
            border: "8px solid #fff59d",
            filter: "chromatic-aberration(-10px)",
          }}
        />,
      )}
      {stage(
        <>
          <View style={{ ...bars, rotate: "60deg", filter: "chromatic-aberration(4px)" }} />
          <View
            style={{
              ...bars,
              position: "absolute",
              scale: 0.5,
              opacity: 0.6,
              filter: "chromatic-aberration(8px)",
            }}
          />
        </>,
        { position: "relative", flexDirection: "column" },
      )}
      {stage(
        <View
          style={{
            width: 72,
            height: 72,
            borderRadius: 12,
            backdropFilter: "chromatic-aberration(5px)",
            background: "rgb(255 255 255 / 10%)",
          }}
        />,
        { background: bars.background + ", #37474f" },
      )}
    </View>
  );
}
