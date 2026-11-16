export const composition = { width: 204, height: 132, fps: 4, duration: 2 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "/tests/resources/photo.png" }) },
};
const CASES = [
  { size: 24, fit: "none", position: "left top", x: 8, y: 8, key: "c0" },
  { size: 24, fit: "none", position: "right bottom", x: 196, y: 8, key: "c1" },
  { size: 24, fit: "scale-down", position: "left top", x: 384, y: 8, key: "c2" },
  { size: 24, fit: "scale-down", position: "right bottom", x: 572, y: 8, key: "c3" },
  { size: 96, fit: "none", position: "left top", x: 8, y: 124, key: "c4" },
  { size: 96, fit: "none", position: "right bottom", x: 196, y: 124, key: "c5" },
  { size: 96, fit: "scale-down", position: "left top", x: 384, y: 124, key: "c6" },
  { size: 96, fit: "scale-down", position: "right bottom", x: 572, y: 124, key: "c7" },
];
const STATES = [0, 2, 4, 7, 5, 0, 7, 2];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const frame = Math.floor(seconds * 4);
  const active = { ...CASES[STATES[frame]], x: 8, y: 8, key: "sample" };
  return (
    <View style={{ width: 204, height: 132, background: "white" }}>
      {[active].map((c) => (
        <Image
          key={c.key}
          src="resource://picture"
          style={{
            position: "absolute",
            left: c.x,
            top: c.y,
            width: c.size,
            height: c.size,
            objectFit: c.fit,
            objectPosition: c.position,
            imageRendering: "pixelated",
            background: "#eef3f8",
          }}
        />
      ))}
    </View>
  );
}
