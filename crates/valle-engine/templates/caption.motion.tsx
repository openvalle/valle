// The standard caption template, which a caption track without a presenter draws each caption
// with: its `caption` ({ text, runs }) in the style its props give, coming and going as
// `animation` says over `transition` seconds. With runs, each lights up at its start; without,
// the characters light up evenly over the caption.

export const composition = { width: 1920, height: 1080, fps: 30, duration: 10 };

export const role = captionPresenter({ intro: 0, outro: 0 });

export const controls = {
  props: {
    fontFamily: string({ default: "sans-serif", label: "Font" }),
    fontSize: number({ default: 60, min: 1, label: "Font size" }),
    fontWeight: number({ default: 600, min: 1, max: 1000, label: "Weight" }),
    textColor: color({ default: "#FFFFFF", label: "Text" }),
    highlight: color({ default: "#FFD23F", label: "Highlight" }),
    strokeWidth: number({ default: 0, min: 0, label: "Stroke width" }),
    strokeColor: color({ default: "#14141A", label: "Stroke" }),
    shadow: boolean({ default: true, label: "Shadow" }),
    background: select({ default: "box", values: ["box", "bar", "none"], label: "Background" }),
    backgroundColor: color({ default: "#000000A6", label: "Background color" }),
    position: select({ default: "bottom", values: ["bottom", "top", "middle"], label: "Position" }),
    margin: number({ default: 90, min: 0, label: "Margin" }),
    maxWidth: number({ default: 1500, min: 1, label: "Max width" }),
    animation: select({
      default: "none",
      values: ["karaoke", "typewriter", "fade", "pop", "slide", "none"],
      label: "Animation",
    }),
    transition: number({ default: 0.2, min: 0, label: "In/out seconds" }),
  },
};

export default function Caption({
  caption,
  fontFamily,
  fontSize,
  fontWeight,
  textColor,
  highlight,
  strokeWidth,
  strokeColor,
  shadow,
  background,
  backgroundColor,
  position,
  margin,
  maxWidth,
  animation,
  transition,
}) {
  const { clipSeconds } = useCurrentTime();
  const { width, height, clipDuration } = useComposition();
  // When each character lights up: at its run's start, or evenly over the caption, finishing a
  // little before it ends.
  const runs = caption.runs;
  const offsets = [];
  let offset = 0;
  for (const r of runs) {
    offsets.push(offset);
    offset += r.text.length;
  }
  const lights = (unit) => {
    if (runs.length === 0) {
      return (unit.index * Math.max(clipDuration - 0.3, 0.1)) / Math.max(unit.count, 1);
    }
    let k = 0;
    while (k + 1 < runs.length && offsets[k + 1] <= unit.start) k++;
    return runs[k].start;
  };
  const span = Math.max(transition, 0.001);
  const enter = clamp(clipSeconds / span, 0, 1);
  const leave = clamp((clipDuration - clipSeconds) / span, 0, 1);
  const shown = Math.min(enter, leave);
  const moves = animation === "fade" || animation === "slide" || animation === "pop";
  const scale =
    animation === "pop" ? interpolate(enter, [0, 1], [0.82, 1], { easing: "easeOutBack" }) : 1;
  const rise = animation === "slide" ? (1 - shown) * 36 : 0;
  const bar = background === "bar";
  // Bottom captions grow upward from `margin` above the bottom edge.
  const place =
    position === "top"
      ? { top: margin }
      : position === "middle"
        ? { top: height / 2 - fontSize }
        : { bottom: margin };
  return (
    <View
      style={{
        position: "absolute",
        left: 0,
        ...place,
        width,
        display: "flex",
        justifyContent: "center",
        opacity: moves ? shown : 1,
        transform: `translateY(${rise}px) scale(${scale})`,
      }}
    >
      <View
        style={{
          maxWidth: bar ? width : maxWidth,
          width: bar ? width : "auto",
          padding: bar ? "14px 210px" : "12px 30px",
          borderRadius: bar ? 0 : 18,
          backgroundColor: background === "none" ? "transparent" : backgroundColor,
        }}
      >
        <Text
          split="char"
          perUnit={(unit) => {
            const lit = clipSeconds >= lights(unit);
            return {
              color: animation === "karaoke" && lit ? highlight : textColor,
              opacity: animation === "typewriter" && !lit ? 0 : 1,
            };
          }}
          style={{
            fontFamily,
            fontSize,
            fontWeight,
            lineHeight: 1.3,
            textAlign: "center",
            whiteSpace: "pre-line",
            color: textColor,
            WebkitTextStroke: `${strokeWidth}px ${strokeColor}`,
            textShadow: shadow ? "0px 4px 14px #000000B0" : "none",
          }}
        >
          {caption.text}
        </Text>
      </View>
    </View>
  );
}
