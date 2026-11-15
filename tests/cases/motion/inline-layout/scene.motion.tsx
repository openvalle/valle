export const composition = { width: 704, height: 360, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View
      key="canvas"
      style={{
        width: 704,
        height: 360,
        background: "repeating-linear-gradient(90deg, #e3f0f8 0 16px, #fff 16px 32px)",
        padding: 16,
        display: "grid",
        gridTemplateColumns: "1fr 1fr",
        gap: 16,
      }}
    >
      {["inline-block", "inline-flex", "inline-grid", "inline-block"].map((display, i) => (
        <View
          key={`card${i}`}
          style={{
            minWidth: 0,
            border: "2px solid #89a9c4",
            borderRadius: 16,
            padding: 12,
            overflow: "hidden",
            background: i === 3 ? "#ffffff88" : "#ffffff",
            backdropFilter: i === 3 ? "blur(4px)" : "none",
            mixBlendMode: i === 3 ? "multiply" : "normal",
            opacity: i === 3 ? 0.85 : 1,
            transform: i === 1 ? `rotate(${seconds - 1}deg)` : "none",
            maskImage:
              i === 2
                ? "linear-gradient(90deg, transparent, black 12%, black 88%, transparent)"
                : "none",
          }}
        >
          <Text
            style={{
              width: "100%",
              fontSize: i === 0 ? "large" : "x-large",
              lineHeight: "40px",
              color: "#17365d",
              textDecoration: i === 2 ? "underline wavy #2b71b8" : "none",
            }}
          >
            State{" "}
            <Span
              style={{
                fontVariantPosition: "super",
                fontSize: "smaller",
                verticalAlign: `${seconds * 3}px`,
              }}
            >
              12Q?
            </Span>{" "}
            <View
              style={{
                display,
                width: 50 + seconds * 24,
                padding: 4,
                gap: 3,
                gridTemplateColumns: "1fr",
                background: "#ffe7ad",
                borderRadius: 5,
                verticalAlign: i === 1 ? "middle" : "baseline",
                filter: i === 2 ? "drop-shadow(1px 2px 1px #17365d66)" : "none",
              }}
            >
              <Text style={{ fontSize: "small", lineHeight: "20px", color: "#36556d" }}>READY</Text>
            </View>{" "}
            words wrap with the badge.
          </Text>
        </View>
      ))}
    </View>
  );
}
