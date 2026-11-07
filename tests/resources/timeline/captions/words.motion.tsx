export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export const role = captionPresenter({ intro: 0.2, outro: 0.2 });
export const controls = { props: { accent: color({ default: "#ff00ff" }) } };

export default function Words({ caption, accent }) {
  const { clipSeconds } = useCurrentTime();
  const { clipDuration } = useComposition();
  const opacity = Math.min(
    clamp(clipSeconds / 0.2, 0, 1),
    clamp((clipDuration - clipSeconds) / 0.2, 0, 1),
  );
  return (
    <View
      style={{
        position: "absolute",
        inset: 0,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        gap: 12,
        opacity,
      }}
    >
      {caption.runs.map((run, i) => (
        <Text
          key={i}
          style={{
            fontSize: 32,
            padding: "8px 12px",
            borderRadius: 12,
            color: "white",
            background: clipSeconds >= run.start ? accent : "#334155",
          }}
        >
          {run.text}
        </Text>
      ))}
    </View>
  );
}
