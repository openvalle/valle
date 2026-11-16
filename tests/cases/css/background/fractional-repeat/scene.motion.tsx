export const composition = { width: 592, height: 352, fps: 1, duration: 1 };

const CASES = [
  { width: 97, repeat: "round", origin: "border-box", x: 8, y: 8, key: "c0" },
  { width: 97, repeat: "round", origin: "content-box", x: 152, y: 8, key: "c1" },
  { width: 97, repeat: "space", origin: "border-box", x: 296, y: 8, key: "c2" },
  { width: 97, repeat: "space", origin: "content-box", x: 440, y: 8, key: "c3" },
  { width: 97, repeat: "round space", origin: "border-box", x: 8, y: 120, key: "c4" },
  { width: 97, repeat: "round space", origin: "content-box", x: 152, y: 120, key: "c5" },
  { width: 112, repeat: "round", origin: "border-box", x: 296, y: 120, key: "c6" },
  { width: 112, repeat: "round", origin: "content-box", x: 440, y: 120, key: "c7" },
  { width: 112, repeat: "space", origin: "border-box", x: 8, y: 232, key: "c8" },
  { width: 112, repeat: "space", origin: "content-box", x: 152, y: 232, key: "c9" },
  { width: 112, repeat: "round space", origin: "border-box", x: 296, y: 232, key: "c10" },
  { width: 112, repeat: "round space", origin: "content-box", x: 440, y: 232, key: "c11" },
];
export default function Scene() {
  return (
    <View style={{ width: 592, height: 352, background: "white" }}>
      {CASES.map((c) => (
        <View
          key={c.key}
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: c.width,
            height: 85,
            padding: 6,
            border: "3px solid #17365d",
            backgroundColor: "#eef3f8",
            backgroundImage: "linear-gradient(90deg, #2463a0 50%, #e0b060 50%)",
            backgroundSize: "23.5px 17.5px",
            backgroundRepeat: c.repeat,
            backgroundOrigin: c.origin,
          }}
        />
      ))}
    </View>
  );
}
