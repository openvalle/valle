export const composition = { width: 140, height: 116, fps: 4, duration: 2 };

const CASES = [
  { position: "left top", repeat: "round no-repeat", x: 8, y: 8, key: "c0" },
  { position: "left top", repeat: "no-repeat round", x: 132, y: 8, key: "c1" },
  { position: "center", repeat: "round no-repeat", x: 256, y: 8, key: "c2" },
  { position: "center", repeat: "no-repeat round", x: 8, y: 108, key: "c3" },
  { position: "right bottom", repeat: "round no-repeat", x: 132, y: 108, key: "c4" },
  { position: "right bottom", repeat: "no-repeat round", x: 256, y: 108, key: "c5" },
];
const STATES = [0, 2, 3, 5, 1, 0, 5, 2];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const frame = Math.floor(seconds * 4);
  const active = { ...CASES[STATES[frame]], x: 8, y: 8, key: "sample" };
  return (
    <View style={{ width: 140, height: 116, background: "white" }}>
      {[active].map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: 100,
            height: 76,
            backgroundImage: "linear-gradient(90deg,#2463a0 50%,#e0b060 50%)",
            backgroundSize: "28px 22px",
            backgroundPosition: c.position,
            backgroundRepeat: c.repeat,
            backgroundColor: "#eef3f8",
          }}
        />
      ))}
    </View>
  );
}
