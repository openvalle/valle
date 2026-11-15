export const composition = { width: 740, height: 384, fps: 1, duration: 1 };
const CASES = ["horizontal-tb", "vertical-rl", "vertical-lr"].flatMap((writing) =>
  [false, true].flatMap((rtl) =>
    ["absolute", "fixed"].map((position) => ({ writing, rtl, position })),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 740, height: 384, background: "white" }}>
      {CASES.map((c, i) => (
        <View
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 4) * 182,
            top: 12 + Math.floor(i / 4) * 124,
            width: 160,
            height: 100,
            padding: 8,
            border: "4px solid #17365d",
            background: "#eef3f8",
            transform: "translate(0px, 0px)",
            writingMode: c.writing,
            direction: c.rtl ? "rtl" : "ltr",
          }}
        >
          <View key="static" style={{ position: "static" }}>
            <View
              key="item"
              style={{
                position: c.position,
                insetInlineStart: 12,
                insetBlockEnd: 8,
                inlineSize: 40,
                blockSize: 20,
                background: "#2463a0",
              }}
            />
          </View>
        </View>
      ))}
    </View>
  );
}
