export const composition = { width: 388, height: 216, fps: 1, duration: 1 };

const CASES = [
  { position: "left top", repeat: "round no-repeat", x: 8, y: 8, key: "c0" },
  { position: "left top", repeat: "no-repeat round", x: 132, y: 8, key: "c1" },
  { position: "center", repeat: "round no-repeat", x: 256, y: 8, key: "c2" },
  { position: "center", repeat: "no-repeat round", x: 8, y: 108, key: "c3" },
  { position: "right bottom", repeat: "round no-repeat", x: 132, y: 108, key: "c4" },
  { position: "right bottom", repeat: "no-repeat round", x: 256, y: 108, key: "c5" },
];
export default function Scene() {
  return (
    <View style={{ width: 388, height: 216, background: "white" }}>
      {CASES.map((c) => (
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
