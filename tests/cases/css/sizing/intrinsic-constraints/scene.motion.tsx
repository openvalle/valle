export const composition = { width: 752, height: 568, fps: 1, duration: 1 };

const CASES = [
  {
    mode: "min-content",
    limit: 90,
    axis: "row",
    sizing: { width: "min-content", maxWidth: 90 },
    x: 8,
    y: 8,
    key: "c0",
  },
  {
    mode: "min-content",
    limit: 90,
    axis: "column",
    sizing: { height: "min-content", maxHeight: 90 },
    x: 192,
    y: 8,
    key: "c1",
  },
  {
    mode: "min-content",
    limit: 150,
    axis: "row",
    sizing: { width: "min-content", maxWidth: 150 },
    x: 376,
    y: 8,
    key: "c2",
  },
  {
    mode: "min-content",
    limit: 150,
    axis: "column",
    sizing: { height: "min-content", maxHeight: 150 },
    x: 560,
    y: 8,
    key: "c3",
  },
  {
    mode: "max-content",
    limit: 90,
    axis: "row",
    sizing: { width: "max-content", maxWidth: 90 },
    x: 8,
    y: 192,
    key: "c4",
  },
  {
    mode: "max-content",
    limit: 90,
    axis: "column",
    sizing: { height: "max-content", maxHeight: 90 },
    x: 192,
    y: 192,
    key: "c5",
  },
  {
    mode: "max-content",
    limit: 150,
    axis: "row",
    sizing: { width: "max-content", maxWidth: 150 },
    x: 376,
    y: 192,
    key: "c6",
  },
  {
    mode: "max-content",
    limit: 150,
    axis: "column",
    sizing: { height: "max-content", maxHeight: 150 },
    x: 560,
    y: 192,
    key: "c7",
  },
  {
    mode: "fit-content",
    limit: 90,
    axis: "row",
    sizing: { width: "fit-content", maxWidth: 90 },
    x: 8,
    y: 376,
    key: "c8",
  },
  {
    mode: "fit-content",
    limit: 90,
    axis: "column",
    sizing: { height: "fit-content", maxHeight: 90 },
    x: 192,
    y: 376,
    key: "c9",
  },
  {
    mode: "fit-content",
    limit: 150,
    axis: "row",
    sizing: { width: "fit-content", maxWidth: 150 },
    x: 376,
    y: 376,
    key: "c10",
  },
  {
    mode: "fit-content",
    limit: 150,
    axis: "column",
    sizing: { height: "fit-content", maxHeight: 150 },
    x: 560,
    y: 376,
    key: "c11",
  },
];
export default function Scene() {
  return (
    <View style={{ width: 752, height: 568, background: "white" }}>
      {CASES.map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 168,
            height: 168,
            background: "#eef3f8",
          }}
        >
          <View
            style={{
              display: "flex",
              flexDirection: c.axis,
              flexWrap: "wrap",
              gap: 8,
              ...c.sizing,
              background: "#dbe9f5",
            }}
          >
            <View style={{ width: 44, height: 44, flexShrink: 0, background: "#2463a0" }} />
            <View style={{ width: 68, height: 68, flexShrink: 0, background: "#3b856c" }} />
          </View>
        </View>
      ))}
    </View>
  );
}
