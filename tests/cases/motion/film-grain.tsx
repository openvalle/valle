export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// film-grain(): fine grain over a gradient, strong, coarse cells, another seed, gentle near black
// and white, scaled with its box, and on what is behind a box.
export default function Scene() {
  const ramp = "linear-gradient(90deg, #000, #e57373 30%, #3949ab 70%, #fff)";
  const tile = (filter: string, style?: object) => (
    <View style={{ width: 120, height: 56, background: ramp, filter, ...style }} />
  );
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 4 }}>
      {tile("film-grain(1 0.3 1px)")}
      {tile("film-grain(1 1 1px)")}
      {tile("film-grain(1 0.8 6px)")}
      {tile("film-grain(2 0.8 6px)")}
      {tile("film-grain(7 0.6 2px)", { scale: 1.5, rotate: "8deg", borderRadius: 12 })}
      <View style={{ width: 120, height: 56, background: ramp }}>
        <View
          style={{
            margin: 10,
            height: 36,
            borderRadius: 18,
            backdropFilter: "film-grain(3 1 3px)",
          }}
        />
      </View>
    </View>
  );
}
