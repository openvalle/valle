export const composition = { width: 320, height: 180, fps: 4, duration: 1 };

export const controls = { props: { accent: color({ default: "#f472b6" }) } };

// A caption presenter: each run in a pill of `accent` that pops in as it is said, over a 0.25 s
// intro and outro.
export const role = captionPresenter({ intro: 0.25, outro: 0.25 });

export default function Pop({ caption, accent }) {
  const { clipSeconds } = useCurrentTime();
  const { clipDuration } = useComposition();
  const shown = Math.min(
    clamp(clipSeconds / 0.25, 0, 1),
    clamp((clipDuration - clipSeconds) / 0.25, 0, 1),
  );
  return (
    <View
      style={{
        position: "absolute",
        left: 0,
        right: 0,
        top: 16,
        display: "flex",
        justifyContent: "center",
        gap: 6,
        opacity: shown,
      }}
    >
      {caption.runs.map((r, i) => (
        <Text
          key={i}
          style={{
            padding: "2px 8px",
            borderRadius: 8,
            fontSize: 14,
            color: "white",
            background: clipSeconds >= r.start ? accent : "#475569",
          }}
        >
          {r.text}
        </Text>
      ))}
    </View>
  );
}
