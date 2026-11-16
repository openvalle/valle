export const composition = { width: 768, height: 248, fps: 1, duration: 1 };

const CASES = [
  {
    pos: { backgroundPositionX: "left", backgroundPositionY: "top", backgroundPosition: "center" },
    x: 8,
    y: 8,
    key: "c0",
  },
  {
    pos: {
      backgroundPositionX: "left",
      backgroundPositionY: "bottom, center",
      backgroundPosition: "center",
    },
    x: 196,
    y: 8,
    key: "c1",
  },
  {
    pos: {
      backgroundPositionX: "right, center",
      backgroundPositionY: "top",
      backgroundPosition: "center",
    },
    x: 384,
    y: 8,
    key: "c2",
  },
  {
    pos: {
      backgroundPositionX: "right, center",
      backgroundPositionY: "bottom, center",
      backgroundPosition: "center",
    },
    x: 572,
    y: 8,
    key: "c3",
  },
  {
    pos: { backgroundPosition: "center", backgroundPositionX: "left", backgroundPositionY: "top" },
    x: 8,
    y: 124,
    key: "c4",
  },
  {
    pos: {
      backgroundPosition: "center",
      backgroundPositionX: "left",
      backgroundPositionY: "bottom, center",
    },
    x: 196,
    y: 124,
    key: "c5",
  },
  {
    pos: {
      backgroundPosition: "center",
      backgroundPositionX: "right, center",
      backgroundPositionY: "top",
    },
    x: 384,
    y: 124,
    key: "c6",
  },
  {
    pos: {
      backgroundPosition: "center",
      backgroundPositionX: "right, center",
      backgroundPositionY: "bottom, center",
    },
    x: 572,
    y: 124,
    key: "c7",
  },
];
export default function Scene() {
  return (
    <View style={{ width: 768, height: 248, background: "white" }}>
      {CASES.map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 92,
            backgroundColor: "#eef3f8",
            backgroundImage: "linear-gradient(#2463a0,#2463a0),linear-gradient(#e0b060,#e0b060)",
            backgroundSize: "32px 24px, 60px 40px",
            backgroundRepeat: "no-repeat",
            ...c.pos,
          }}
        />
      ))}
    </View>
  );
}
