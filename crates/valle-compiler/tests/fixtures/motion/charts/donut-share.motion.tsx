export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

export const controls = {
  props: {
    ink: color({ default: "#e2e8f0" }),
    mute: color({ default: "#64748b" }),
  },
};

const SHARE = [
  { id: "core", label: "Core", value: 42, color: "#38bdf8" },
  { id: "cloud", label: "Cloud", value: 26, color: "#818cf8" },
  { id: "edge", label: "Edge", value: 18, color: "#2dd4bf" },
  { id: "labs", label: "Labs", value: 14, color: "#f472b6" },
];
const SLICES = pie(
  SHARE.map((item) => item.value),
  { startAngle: deg(-90), endAngle: deg(-90) + TAU, padAngle: deg(2.5) },
);
const CX = 645;
const CY = 600;
const OUTER = 282;
const BOX_W = 186;
const BOX_H = 75;

function layoutCallouts(slices) {
  const layouts = slices.map((slice) => {
    const cosine = Math.cos(slice.midAngle);
    const sine = Math.sin(slice.midAngle);
    const right = cosine >= 0;
    const rim = point(CX + (OUTER + 15) * cosine, CY + (OUTER + 15) * sine);
    const elbow = point(CX + (OUTER + 78) * cosine, CY + (OUTER + 78) * sine);
    const joinX = right ? elbow.x + 27 : elbow.x - 27;
    const join = point(joinX, elbow.y);
    return {
      rim,
      elbow,
      join,
      textLeft: right ? joinX + 12 : joinX - 12 - BOX_W,
      textTop: elbow.y - BOX_H / 2,
      align: right ? "left" : "right",
      right,
    };
  });
  for (const right of [true, false]) {
    const order = layouts
      .map((callout, index) => index)
      .filter((index) => layouts[index].right === right)
      .sort((a, b) => layouts[a].textTop - layouts[b].textTop);
    for (let slot = 1; slot < order.length; slot++) {
      const previous = layouts[order[slot - 1]];
      const current = layouts[order[slot]];
      const minTop = previous.textTop + BOX_H + 18;
      if (current.textTop < minTop) {
        const shift = minTop - current.textTop;
        current.textTop += shift;
        current.join = point(current.join.x, current.join.y + shift);
      }
    }
  }
  return layouts;
}

const CALLOUTS = layoutCallouts(SLICES);

function Slice(ctx, { index, color }) {
  const slice = SLICES[index];
  const opened = interpolate(ctx.progress - index * 0.05, [0, 0.55], [0, 1], {
    easing: "easeOut",
  });
  const hole = interpolate(ctx.progress, [0.12, 0.55], [0, 138], { easing: "easeOut" });
  return (
    <Path
      key="wedge"
      d={sector({
        center: point(CX, CY),
        inner: hole,
        outer: OUTER,
        start: slice.startAngle,
        end: interpolate(opened, [0, 1], [slice.startAngle, slice.endAngle]),
        cornerRadius: 15,
      })}
      fill={color}
    />
  );
}

function Callout(ctx, { index, label, color }) {
  const layout = CALLOUTS[index];
  const fade = interpolate(ctx.progress - 0.28 - index * 0.04, [0, 0.35], [0, 1], {
    easing: "easeOut",
  });
  const leader =
    layout.join.y === layout.elbow.y
      ? line([layout.rim, layout.elbow, layout.join])
      : line([layout.rim, layout.elbow, point(layout.join.x, layout.elbow.y), layout.join]);
  return (
    <View key="callout" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080, opacity: fade }}>
      <Path
        key="leader"
        style={{ position: "absolute", left: 0, top: 0 }}
        d={leader}
        fill="none"
        stroke={color}
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <Text
        key="name"
        className="absolute"
        style={{
          left: layout.textLeft,
          top: layout.textTop,
          width: BOX_W,
          fontSize: 30,
          color,
          textAlign: layout.align,
        }}
      >
        {label}
      </Text>
      <Text
        key="share"
        className="absolute"
        style={{
          left: layout.textLeft,
          top: layout.textTop + 36,
          width: BOX_W,
          fontSize: 27,
          color: "#94a3b8",
          textAlign: layout.align,
        }}
      >
        {formatPercent(SLICES[index].fraction, { decimals: 0 })}
      </Text>
    </View>
  );
}

export default function DonutShare(ctx, props) {
  const total = interpolate(ctx.progress, [0.2, 0.75], [0, 1], { easing: "easeOut" });
  return (
    <Scene key="scene" className="relative h-full w-full" style={{ backgroundColor: "#071018" }}>
      <Text key="eyebrow" className="absolute" style={{ left: 108, top: 66, fontSize: 27, letterSpacing: 4.5, color: "#38bdf8" }}>
        MIX / FOUR LINES
      </Text>
      <Text key="title" className="absolute" style={{ left: 108, top: 114, fontSize: 63, color: props.ink }}>
        Share opens as a ring
      </Text>
      {SHARE.map((item, index) => (
        <Slice key={item.id} index={index} color={item.color} />
      ))}
      {SHARE.map((item, index) => (
        <Callout key={`call-${item.id}`} index={index} label={item.label} color={item.color} />
      ))}
      <Text
        key="total-label"
        className="absolute"
        style={{
          left: CX - 120,
          top: CY - 42,
          width: 360,
          fontSize: 24,
          letterSpacing: 3,
          color: props.mute,
          textAlign: "center",
          opacity: total,
        }}
      >
        TOTAL
      </Text>
      <Text
        key="total-value"
        className="absolute"
        style={{
          left: CX - 120,
          top: CY - 6,
          width: 360,
          fontSize: 54,
          color: props.ink,
          textAlign: "center",
          opacity: total,
        }}
      >
        {formatNumber(100 * total, { decimals: 0 })}
      </Text>
    </Scene>
  );
}
