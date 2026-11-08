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
    fontStyle: select({
      default: "normal",
      values: ["normal", "italic", "oblique"],
      label: "Font style",
    }),
    lineHeight: number({ default: 1.3, min: 0, label: "Line height" }),
    letterSpacing: number({ default: 0, label: "Letter spacing" }),
    wordSpacing: number({ default: 0, label: "Word spacing" }),
    textAlign: select({
      default: "center",
      values: ["left", "center", "right", "justify"],
      label: "Alignment",
    }),
    textDecoration: string({ default: "none", label: "Text decoration" }),
    textUnderlineOffset: string({ default: "auto", label: "Underline offset" }),
    textDecorationSkipInk: select({
      default: "auto",
      values: ["auto", "all", "none"],
      label: "Skip glyph ink",
    }),
    textColor: color({ default: "#FFFFFF", label: "Text" }),
    highlight: color({ default: "#FFD23F", label: "Highlight" }),
    strokeWidth: number({ default: 0, min: 0, label: "Stroke width" }),
    strokeColor: color({ default: "#14141A", label: "Stroke" }),
    shadow: boolean({ default: true, label: "Shadow" }),
    textShadow: string({ default: "0px 4px 14px #000000B0", label: "Text shadow" }),
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

// The same basic text properties apply to the whole caption and to individual runs.
function textStyle(s) {
  return {
    fontFamily: s.fontFamily,
    fontSize: s.fontSize,
    fontWeight: s.fontWeight,
    fontStyle: s.fontStyle,
    lineHeight: s.lineHeight,
    letterSpacing: s.letterSpacing,
    wordSpacing: s.wordSpacing,
    color: s.textColor,
    textDecoration: s.textDecoration,
    textUnderlineOffset: s.textUnderlineOffset,
    textDecorationSkipInk: s.textDecorationSkipInk,
    WebkitTextStroke: `${s.strokeWidth}px ${s.strokeColor}`,
    // Keep the fill over the stroke, including overlapping CJK contours.
    paintOrder: "stroke fill",
    textShadow: s.shadow ? s.textShadow : "none",
  };
}

function Words({ run, base, animation, seconds, duration }) {
  for (const [key, value] of Object.entries(run.style ?? {})) {
    if (!Object.hasOwn(base, key)) throw new Error(`caption run.style has no ${key}`);
    if (typeof value !== typeof base[key]) {
      throw new Error(`caption run.style.${key} must be a ${typeof base[key]}`);
    }
  }
  const style = { ...base, ...run.style };
  const activeStyle = (start) => ({
    color: animation === "karaoke" && seconds >= start ? style.highlight : style.textColor,
    visibility: animation === "typewriter" && seconds < start ? "hidden" : "visible",
  });
  // Only untimed karaoke/typewriter captions need character splitting. Static text and timed
  // runs keep one decorating box, including their spaces.
  if (run.start !== undefined || (animation !== "karaoke" && animation !== "typewriter")) {
    return <Span style={{ ...textStyle(style), ...activeStyle(run.start ?? 0) }}>{run.text}</Span>;
  }
  return (
    <Span
      style={{ ...textStyle(style), textDecoration: "none" }}
      split="char"
      perUnit={(unit) => ({
        // Decorations follow the revealed characters, too, not the spaces between them.
        textDecoration: style.textDecoration,
        ...activeStyle((unit.index * Math.max(duration - 0.3, 0.1)) / Math.max(unit.count, 1)),
      })}
    >
      {run.text}
    </Span>
  );
}

export default function Caption({ caption, ...props }) {
  const {
    fontFamily,
    fontSize,
    fontWeight,
    fontStyle,
    lineHeight,
    letterSpacing,
    wordSpacing,
    textColor,
    highlight,
    strokeWidth,
    strokeColor,
    shadow,
    textShadow,
    textDecoration,
    textUnderlineOffset,
    textDecorationSkipInk,
    textAlign,
    background,
    backgroundColor,
    position,
    margin,
    maxWidth,
    animation,
    transition,
  } = props;
  const base = {
    fontFamily,
    fontSize,
    fontWeight,
    fontStyle,
    lineHeight,
    letterSpacing,
    wordSpacing,
    textColor,
    highlight,
    strokeWidth,
    strokeColor,
    shadow,
    textShadow,
    textDecoration,
    textUnderlineOffset,
    textDecorationSkipInk,
  };
  const { clipSeconds } = useCurrentTime();
  const { width, height, clipDuration } = useComposition();
  const span = Math.max(transition, 0.001);
  const enter = clamp(clipSeconds / span, 0, 1);
  const leave = clamp((clipDuration - clipSeconds) / span, 0, 1);
  const shown = Math.min(enter, leave);
  const moves = animation === "fade" || animation === "slide" || animation === "pop";
  const scale =
    animation === "pop"
      ? interpolate(enter, [0, 1], [0.82, 1], { easing: "cubic-bezier(0.34, 1.56, 0.64, 1)" })
      : 1;
  const rise = animation === "slide" ? (1 - shown) * 36 : 0;
  const bar = background === "bar";
  // Bottom captions grow upward from `margin` above the bottom edge.
  const place =
    position === "top"
      ? { top: margin }
      : position === "middle"
        ? { top: height / 2 }
        : { bottom: margin };
  const runs = caption.runs.length ? caption.runs : [{ text: caption.text }];
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
        transform: `${position === "middle" ? "translateY(-50%) " : ""}translateY(${rise}px) scale(${scale})`,
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
          style={{ ...textStyle(base), textAlign, whiteSpace: "pre-line", textDecoration: "none" }}
        >
          {runs.map((run, i) => (
            <Words
              key={i}
              run={run}
              base={base}
              animation={animation}
              seconds={clipSeconds}
              duration={clipDuration}
            />
          ))}
        </Text>
      </View>
    </View>
  );
}
