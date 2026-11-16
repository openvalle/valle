export const composition = { width: 876, height: 508, fps: 1, duration: 1 };
const CASES = [86, 88].flatMap((width) =>
  ["normal", "nowrap", "pre-wrap"].flatMap((space) =>
    ["normal", "break-all"].flatMap((word) =>
      ["normal", "anywhere"].map((overflow) => ({ width, space, word, overflow })),
    ),
  ),
);
export default function Scene() {
  return (
    <View style={{ width: 876, height: 508, background: "white" }}>
      {CASES.map((c, i) => (
        <Text
          key={`c${i}`}
          style={{
            position: "absolute",
            left: 12 + (i % 6) * 144,
            top: 12 + Math.floor(i / 6) * 124,
            width: c.width,
            height: 112,
            fontSize: 24,
            lineHeight: "32px",
            color: "#17365d",
            background: "#eef3f8",
            overflow: "hidden",
            whiteSpace: c.space,
            wordBreak: c.word,
            overflowWrap: c.overflow,
          }}
        >
          MMMMMMMM
        </Text>
      ))}
    </View>
  );
}
