export const composition = { width: 1216, height: 556, fps: 1, duration: 1 };

const CASES = [
  {
    origin: "left top",
    transform: "translate(12px, 8px) rotate(30deg) scale(-1, 0.75)",
    overflow: "visible",
    x: 8,
    y: 8,
    key: "c0",
  },
  {
    origin: "left top",
    transform: "translate(12px, 8px) rotate(30deg) scale(-1, 0.75)",
    overflow: "hidden",
    x: 308,
    y: 8,
    key: "c1",
  },
  {
    origin: "left top",
    transform: "scale(-1, 0.75) rotate(30deg) translate(12px, 8px)",
    overflow: "visible",
    x: 608,
    y: 8,
    key: "c2",
  },
  {
    origin: "left top",
    transform: "scale(-1, 0.75) rotate(30deg) translate(12px, 8px)",
    overflow: "hidden",
    x: 908,
    y: 8,
    key: "c3",
  },
  {
    origin: "center",
    transform: "translate(12px, 8px) rotate(30deg) scale(-1, 0.75)",
    overflow: "visible",
    x: 8,
    y: 188,
    key: "c4",
  },
  {
    origin: "center",
    transform: "translate(12px, 8px) rotate(30deg) scale(-1, 0.75)",
    overflow: "hidden",
    x: 308,
    y: 188,
    key: "c5",
  },
  {
    origin: "center",
    transform: "scale(-1, 0.75) rotate(30deg) translate(12px, 8px)",
    overflow: "visible",
    x: 608,
    y: 188,
    key: "c6",
  },
  {
    origin: "center",
    transform: "scale(-1, 0.75) rotate(30deg) translate(12px, 8px)",
    overflow: "hidden",
    x: 908,
    y: 188,
    key: "c7",
  },
  {
    origin: "right bottom",
    transform: "translate(12px, 8px) rotate(30deg) scale(-1, 0.75)",
    overflow: "visible",
    x: 8,
    y: 368,
    key: "c8",
  },
  {
    origin: "right bottom",
    transform: "translate(12px, 8px) rotate(30deg) scale(-1, 0.75)",
    overflow: "hidden",
    x: 308,
    y: 368,
    key: "c9",
  },
  {
    origin: "right bottom",
    transform: "scale(-1, 0.75) rotate(30deg) translate(12px, 8px)",
    overflow: "visible",
    x: 608,
    y: 368,
    key: "c10",
  },
  {
    origin: "right bottom",
    transform: "scale(-1, 0.75) rotate(30deg) translate(12px, 8px)",
    overflow: "hidden",
    x: 908,
    y: 368,
    key: "c11",
  },
];
export default function Scene() {
  return (
    <View style={{ width: 1216, height: 556, background: "white" }}>
      {CASES.map((c) => (
        <View
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 92,
            background: "#eef3f8",
            borderRadius: 18,
            overflow: c.overflow,
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 42,
              top: 18,
              width: 96,
              height: 62,
              background: "#2463a0",
              transform: c.transform,
              transformOrigin: c.origin,
              translate: "4px 2px",
              rotate: "10deg",
              scale: "1.1 .9",
            }}
          >
            <View style={{ width: 22, height: 30, background: "#e0b060" }} />
          </View>
        </View>
      ))}
    </View>
  );
}
