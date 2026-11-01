export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// lens-distortion(): a grid bulged as a barrel lens shows it, pinched as a pincushion one does,
// bent at the edges alone by k2, turned with its box, and what is behind a box.
export default function Scene() {
  const grid =
    "repeating-linear-gradient(90deg, #ffca28 0 2px, transparent 2px 12px), " +
    "repeating-linear-gradient(#ffca28 0 2px, transparent 2px 12px), #3949ab";
  const tile = (filter: string, style?: object) => (
    <View style={{ width: 116, height: 116, background: grid, filter, ...style }} />
  );
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 4 }}>
      {tile("lens-distortion(0.4 0)")}
      {tile("lens-distortion(-0.3 0)")}
      {tile("lens-distortion(0 0.5)", { rotate: "20deg", scale: 0.8, borderRadius: 20 })}
      <View style={{ width: 116, height: 116, background: grid }}>
        <View
          style={{
            margin: 18,
            width: 80,
            height: 80,
            borderRadius: 40,
            backdropFilter: "lens-distortion(0.5 0.2)",
            border: "2px solid white",
          }}
        />
      </View>
    </View>
  );
}
