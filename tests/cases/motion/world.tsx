export const composition = { width: 192, height: 128, fps: 4, duration: 1 };

const TILES = [0, 1, 2, 3, 4, 5].flatMap((row) =>
  [0, 1, 2, 3, 4, 5, 6, 7].map((col) => ({ row, col })),
);

// A checkered world the camera zooms into and turns, under a label fixed on the screen.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const camera = {
    center: { x: interpolate(seconds, [0, 1], [96, 120]), y: 64 },
    zoom: interpolate(seconds, [0, 1], [1, 1.75]),
    rotation: interpolate(seconds, [0, 1], [0, 30]),
  };
  return (
    <View style={{ width: 192, height: 128, background: "#fff8e1" }}>
      <World camera={camera}>
        {TILES.map(({ row, col }) => (
          <View
            key={`${row}-${col}`}
            style={{
              position: "absolute",
              left: col * 24,
              top: row * 24 - 8,
              width: 24,
              height: 24,
              background: (row + col) % 2 === 0 ? "#26a69a" : "#ffca28",
            }}
          />
        ))}
      </World>
      <View
        style={{
          position: "absolute",
          left: 8,
          top: 8,
          width: 40,
          height: 16,
          borderRadius: 8,
          background: "#ef5350",
        }}
      />
    </View>
  );
}
