export const composition = { width: 204, height: 144, fps: 4, duration: 2 };

const CASES = [
  {
    overflow: "visible",
    transform: "none",
    shadow: "4px 4px 0px #3b856c, -4px -4px 0px #c05c30",
    x: 8,
    y: 8,
    key: "c0",
  },
  {
    overflow: "visible",
    transform: "none",
    shadow: "inset 5px 4px 0px #3b856c, inset -4px -3px 0px #c05c30",
    x: 196,
    y: 8,
    key: "c1",
  },
  {
    overflow: "visible",
    transform: "none",
    shadow: "3px 4px 3px #8c407c, inset 4px 3px 2px #3b856c",
    x: 384,
    y: 8,
    key: "c2",
  },
  {
    overflow: "visible",
    transform: "rotate(9deg)",
    shadow: "4px 4px 0px #3b856c, -4px -4px 0px #c05c30",
    x: 572,
    y: 8,
    key: "c3",
  },
  {
    overflow: "visible",
    transform: "rotate(9deg)",
    shadow: "inset 5px 4px 0px #3b856c, inset -4px -3px 0px #c05c30",
    x: 8,
    y: 136,
    key: "c4",
  },
  {
    overflow: "visible",
    transform: "rotate(9deg)",
    shadow: "3px 4px 3px #8c407c, inset 4px 3px 2px #3b856c",
    x: 196,
    y: 136,
    key: "c5",
  },
  {
    overflow: "hidden",
    transform: "none",
    shadow: "4px 4px 0px #3b856c, -4px -4px 0px #c05c30",
    x: 384,
    y: 136,
    key: "c6",
  },
  {
    overflow: "hidden",
    transform: "none",
    shadow: "inset 5px 4px 0px #3b856c, inset -4px -3px 0px #c05c30",
    x: 572,
    y: 136,
    key: "c7",
  },
  {
    overflow: "hidden",
    transform: "none",
    shadow: "3px 4px 3px #8c407c, inset 4px 3px 2px #3b856c",
    x: 8,
    y: 264,
    key: "c8",
  },
  {
    overflow: "hidden",
    transform: "rotate(9deg)",
    shadow: "4px 4px 0px #3b856c, -4px -4px 0px #c05c30",
    x: 196,
    y: 264,
    key: "c9",
  },
  {
    overflow: "hidden",
    transform: "rotate(9deg)",
    shadow: "inset 5px 4px 0px #3b856c, inset -4px -3px 0px #c05c30",
    x: 384,
    y: 264,
    key: "c10",
  },
  {
    overflow: "hidden",
    transform: "rotate(9deg)",
    shadow: "3px 4px 3px #8c407c, inset 4px 3px 2px #3b856c",
    x: 572,
    y: 264,
    key: "c11",
  },
];
const STATES = [0, 3, 8, 11, 1, 0, 11, 3];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const frame = Math.floor(seconds * 4);
  const active = { ...CASES[STATES[frame]], x: 8, y: 8, key: "sample" };
  return (
    <View style={{ width: 204, height: 144, background: "white" }}>
      {[active].map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 160,
            height: 100,
            borderRadius: 12,
            overflow: c.overflow,
            background: "#eef3f8",
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 6,
              top: 8,
              width: 140,
              height: 76,
              borderRadius: 18,
              outline: "3px dashed #2463a0",
              outlineOffset: 4,
              boxShadow: c.shadow,
              transform: c.transform,
              background: "#e0b060",
            }}
          />
        </View>
      ))}
    </View>
  );
}
