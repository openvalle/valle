export const composition = { width: 204, height: 132, fps: 4, duration: 2 };

const CASES = [
  {
    size: "24px 16px",
    repeat: "no-repeat",
    pos: "left top",
    bg: {
      backgroundImage:
        "linear-gradient(90deg, #2463a0 50%, transparent 50%), linear-gradient(#3b856c 50%, #e0b060 50%)",
      backgroundSize: "24px 16px",
      backgroundRepeat: "no-repeat",
      backgroundPosition: "left top",
    },
    x: 8,
    y: 8,
    key: "c0",
  },
  {
    size: "24px 16px",
    repeat: "no-repeat",
    pos: "right bottom, center",
    bg: {
      backgroundImage:
        "linear-gradient(90deg, #2463a0 50%, transparent 50%), linear-gradient(#3b856c 50%, #e0b060 50%)",
      backgroundSize: "24px 16px",
      backgroundRepeat: "no-repeat",
      backgroundPosition: "right bottom, center",
    },
    x: 196,
    y: 8,
    key: "c1",
  },
  {
    size: "24px 16px",
    repeat: "repeat-x, repeat-y",
    pos: "left top",
    bg: {
      backgroundImage:
        "linear-gradient(90deg, #2463a0 50%, transparent 50%), linear-gradient(#3b856c 50%, #e0b060 50%)",
      backgroundSize: "24px 16px",
      backgroundRepeat: "repeat-x, repeat-y",
      backgroundPosition: "left top",
    },
    x: 384,
    y: 8,
    key: "c2",
  },
  {
    size: "24px 16px",
    repeat: "repeat-x, repeat-y",
    pos: "right bottom, center",
    bg: {
      backgroundImage:
        "linear-gradient(90deg, #2463a0 50%, transparent 50%), linear-gradient(#3b856c 50%, #e0b060 50%)",
      backgroundSize: "24px 16px",
      backgroundRepeat: "repeat-x, repeat-y",
      backgroundPosition: "right bottom, center",
    },
    x: 572,
    y: 8,
    key: "c3",
  },
  {
    size: "24px 16px, 40px 28px",
    repeat: "no-repeat",
    pos: "left top",
    bg: {
      backgroundImage:
        "linear-gradient(90deg, #2463a0 50%, transparent 50%), linear-gradient(#3b856c 50%, #e0b060 50%)",
      backgroundSize: "24px 16px, 40px 28px",
      backgroundRepeat: "no-repeat",
      backgroundPosition: "left top",
    },
    x: 8,
    y: 124,
    key: "c4",
  },
  {
    size: "24px 16px, 40px 28px",
    repeat: "no-repeat",
    pos: "right bottom, center",
    bg: {
      backgroundImage:
        "linear-gradient(90deg, #2463a0 50%, transparent 50%), linear-gradient(#3b856c 50%, #e0b060 50%)",
      backgroundSize: "24px 16px, 40px 28px",
      backgroundRepeat: "no-repeat",
      backgroundPosition: "right bottom, center",
    },
    x: 196,
    y: 124,
    key: "c5",
  },
  {
    size: "24px 16px, 40px 28px",
    repeat: "repeat-x, repeat-y",
    pos: "left top",
    bg: {
      backgroundImage:
        "linear-gradient(90deg, #2463a0 50%, transparent 50%), linear-gradient(#3b856c 50%, #e0b060 50%)",
      backgroundSize: "24px 16px, 40px 28px",
      backgroundRepeat: "repeat-x, repeat-y",
      backgroundPosition: "left top",
    },
    x: 384,
    y: 124,
    key: "c6",
  },
  {
    size: "24px 16px, 40px 28px",
    repeat: "repeat-x, repeat-y",
    pos: "right bottom, center",
    bg: {
      backgroundImage:
        "linear-gradient(90deg, #2463a0 50%, transparent 50%), linear-gradient(#3b856c 50%, #e0b060 50%)",
      backgroundSize: "24px 16px, 40px 28px",
      backgroundRepeat: "repeat-x, repeat-y",
      backgroundPosition: "right bottom, center",
    },
    x: 572,
    y: 124,
    key: "c7",
  },
];
const STATES = [0, 1, 3, 7, 4, 0, 7, 1];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const frame = Math.floor(seconds * 4);
  const active = { ...CASES[STATES[frame]], x: 8, y: 8, key: "sample" };
  return (
    <View style={{ width: 204, height: 132, background: "white" }}>
      {[active].map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 156,
            height: 92,
            border: "4px solid #17365d",
            padding: 8,
            backgroundColor: "#eef3f8",
            backgroundOrigin: "content-box",
            ...c.bg,
          }}
        />
      ))}
    </View>
  );
}
