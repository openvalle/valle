export const composition = { width: 240, height: 80, fps: 4, duration: 2 };

// A caption presenter takes the caption the Timeline gives as its `caption` prop: alone, a
// placeholder whose words come one by one. It slides in, lights each word as it is said and
// fades out.
export const role = captionPresenter({ intro: 0.25, outro: 0.25 });

type Caption = { text: string; runs: { text: string; start: number; end: number }[] };

export default function Scene({ caption }: { caption: Caption }) {
  const { clipSeconds, clipProgress } = useCurrentTime();
  const { clipDuration } = useComposition();
  const shown = interpolate(
    clipSeconds,
    [0, 0.25, clipDuration - 0.25, clipDuration],
    [0, 1, 1, 0],
  );
  return (
    <View
      style={{
        width: 240,
        height: 80,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: "#263238",
      }}
    >
      <Text
        style={{
          width: 220,
          textAlign: "center",
          fontSize: 18,
          fontWeight: 700,
          color: "white",
          opacity: shown,
          transform: `translateY(${(1 - shown) * 12}px)`,
        }}
      >
        {caption.runs.map((run, i) => (
          <Span key={i} style={{ color: clipSeconds >= run.start ? "#ffca28" : "white" }}>
            {(i > 0 ? " " : "") + run.text}
          </Span>
        ))}
      </Text>
      <View
        style={{
          position: "absolute",
          left: 0,
          bottom: 0,
          height: 3,
          width: 240 * clipProgress,
          background: "#ffca28",
        }}
      />
    </View>
  );
}
