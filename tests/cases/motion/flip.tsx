export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

// Three cards moving from a row into a stack and a wide card, each between its boxes in the two
// layout states, scaled with what is in it.
const layouts = defineLayoutStates({
  row: {
    a: { x: 16, y: 56, width: 64, height: 48 },
    b: { x: 96, y: 56, width: 64, height: 48 },
    c: { x: 176, y: 56, width: 64, height: 48 },
  },
  stack: {
    a: { x: 16, y: 16, width: 96, height: 56 },
    b: { x: 16, y: 88, width: 96, height: 56 },
    c: { x: 128, y: 16, width: 112, height: 128 },
  },
});

export default function Scene() {
  const { seconds } = useCurrentTime();
  const f = flip(layouts, "row", "stack", interpolate(seconds, [0, 1], [0, 1.25]));
  const card = (id, color) => (
    <View
      key={id}
      layoutId={id}
      style={{ background: color, borderRadius: 8, padding: 8, layoutTransition: f }}
    >
      <View style={{ width: 24, height: 8, borderRadius: 4, background: "#ffffff" }} />
    </View>
  );
  return (
    <View style={{ width: 256, height: 160 }}>
      {card("a", "#1e88e5")}
      {card("b", "#43a047")}
      {card("c", "#fb8c00")}
    </View>
  );
}
