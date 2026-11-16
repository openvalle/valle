// Logical sides in every writing mode and direction: an absolutely positioned box whose
// insets, margins, paddings and borders are each written per flow-relative side, with a
// different value on each, inside a positioned cell. See the README for the arithmetic.
export const composition = { width: 498, height: 336, fps: 1, duration: 1 };

const CASES = [
  { writing: "horizontal-tb", direction: "ltr", x: 12, y: 12 },
  { writing: "horizontal-tb", direction: "rtl", x: 174, y: 12 },
  { writing: "vertical-rl", direction: "ltr", x: 336, y: 12 },
  { writing: "vertical-rl", direction: "rtl", x: 12, y: 174 },
  { writing: "vertical-lr", direction: "ltr", x: 174, y: 174 },
  { writing: "vertical-lr", direction: "rtl", x: 336, y: 174 },
];

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 498, height: 336, background: "white" }}>
      {CASES.map((c, i) => (
        <View
          key={`c${i}`}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 150,
            height: 150,
            padding: 4,
            border: "8px solid #17365d",
            background: "#eef3f8",
            writingMode: c.writing,
            direction: c.direction,
          }}
        >
          <View
            key="item"
            style={{
              position: "absolute",
              insetInlineStart: 7,
              insetBlockStart: 5,
              inlineSize: 60,
              blockSize: 30,
              marginBlockStart: 3,
              marginBlockEnd: 5,
              marginInlineStart: 9,
              marginInlineEnd: 11,
              paddingBlockStart: 2,
              paddingBlockEnd: 4,
              paddingInlineStart: 6,
              paddingInlineEnd: 8,
              borderBlockStart: "1px solid #c0392b",
              borderBlockEnd: "3px solid #27ae60",
              borderInlineStart: "5px solid #2463a0",
              borderInlineEnd: "7px solid #8e44ad",
              background: "#f2c14e",
            }}
          />
        </View>
      ))}
    </View>
  );
}
