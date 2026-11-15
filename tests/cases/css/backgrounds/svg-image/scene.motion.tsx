export const composition = { width: 360, height: 200, fps: 1, duration: 1 };
export const controls = {
  resources: { tile: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  const tiles = [
    { backgroundSize: "auto", backgroundRepeat: "no-repeat" },
    { backgroundSize: "cover", backgroundRepeat: "no-repeat" },
    { backgroundSize: "40px auto", backgroundRepeat: "repeat" },
    { backgroundSize: "40px 20px", backgroundRepeat: "space" },
    { backgroundSize: "40px 20px", backgroundRepeat: "repeat-x" },
    { backgroundSize: "40px 20px", backgroundRepeat: "round" },
  ];
  return (
    <View style={{ width: 360, height: 200, backgroundColor: "white" }}>
      {tiles.map((tile, i) => (
        <View
          key={`tile-${i}`}
          style={{
            position: "absolute",
            left: 10 + (i % 3) * 120,
            top: 10 + Math.floor(i / 3) * 95,
            width: 100,
            height: 80,
            backgroundColor: "#e8edf2",
            backgroundImage: "url(resource://tile)",
            backgroundPosition: "center",
            borderRadius: i === 2 ? 12 : 0,
            ...tile,
          }}
        />
      ))}
    </View>
  );
}
