export const composition = { width: 768, height: 248, fps: 1, duration: 1 };
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
export default function Scene() {
  return (
    <View style={{ width: 768, height: 248, background: "white" }}>
      {CASES.map((c) => (
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
