export const composition = { width: 768, height: 364, fps: 1, duration: 1 };

const CASES = [
  {
    writing: "horizontal-tb",
    direction: "ltr",
    side: "Left",
    padding: 16,
    edges: { paddingLeft: 8, paddingInlineStart: 16 },
    x: 8,
    y: 8,
    key: "c0",
  },
  {
    writing: "horizontal-tb",
    direction: "ltr",
    side: "Left",
    padding: 8,
    edges: { paddingInlineStart: 16, paddingLeft: 8 },
    x: 196,
    y: 8,
    key: "c1",
  },
  {
    writing: "horizontal-tb",
    direction: "rtl",
    side: "Right",
    padding: 16,
    edges: { paddingRight: 8, paddingInlineStart: 16 },
    x: 384,
    y: 8,
    key: "c2",
  },
  {
    writing: "horizontal-tb",
    direction: "rtl",
    side: "Right",
    padding: 8,
    edges: { paddingInlineStart: 16, paddingRight: 8 },
    x: 572,
    y: 8,
    key: "c3",
  },
  {
    writing: "vertical-rl",
    direction: "ltr",
    side: "Top",
    padding: 16,
    edges: { paddingTop: 8, paddingInlineStart: 16 },
    x: 8,
    y: 124,
    key: "c4",
  },
  {
    writing: "vertical-rl",
    direction: "ltr",
    side: "Top",
    padding: 8,
    edges: { paddingInlineStart: 16, paddingTop: 8 },
    x: 196,
    y: 124,
    key: "c5",
  },
  {
    writing: "vertical-rl",
    direction: "rtl",
    side: "Bottom",
    padding: 16,
    edges: { paddingBottom: 8, paddingInlineStart: 16 },
    x: 384,
    y: 124,
    key: "c6",
  },
  {
    writing: "vertical-rl",
    direction: "rtl",
    side: "Bottom",
    padding: 8,
    edges: { paddingInlineStart: 16, paddingBottom: 8 },
    x: 572,
    y: 124,
    key: "c7",
  },
  {
    writing: "vertical-lr",
    direction: "ltr",
    side: "Top",
    padding: 16,
    edges: { paddingTop: 8, paddingInlineStart: 16 },
    x: 8,
    y: 240,
    key: "c8",
  },
  {
    writing: "vertical-lr",
    direction: "ltr",
    side: "Top",
    padding: 8,
    edges: { paddingInlineStart: 16, paddingTop: 8 },
    x: 196,
    y: 240,
    key: "c9",
  },
  {
    writing: "vertical-lr",
    direction: "rtl",
    side: "Bottom",
    padding: 16,
    edges: { paddingBottom: 8, paddingInlineStart: 16 },
    x: 384,
    y: 240,
    key: "c10",
  },
  {
    writing: "vertical-lr",
    direction: "rtl",
    side: "Bottom",
    padding: 8,
    edges: { paddingInlineStart: 16, paddingBottom: 8 },
    x: 572,
    y: 240,
    key: "c11",
  },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 364, background: "white" }}>
      {CASES.map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 160,
            height: 88,
            writingMode: c.writing,
            direction: c.direction,
            background: "#eef3f8",
            ...c.edges,
          }}
        >
          <View style={{ width: "100%", height: "100%", background: "#2463a0" }} />
        </View>
      ))}
    </View>
  );
}
