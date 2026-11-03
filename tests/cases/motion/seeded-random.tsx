export const composition = { width: 256, height: 128, fps: 1, duration: 1 };

// Dots placed by seeded random numbers: positions in the frame, sizes in a range of their own.
const xs = seededRandom(1, 24, [8, 232]);
const ys = seededRandom(2, 24, [8, 104]);
const sizes = seededRandom(3, 24, [4, 16]);

export default function Scene() {
  return (
    <View style={{ width: 256, height: 128 }}>
      {xs.map((x, i) => (
        <View
          style={{
            position: "absolute",
            left: x,
            top: ys[i],
            width: sizes[i],
            height: sizes[i],
            borderRadius: sizes[i] / 2,
            background: i % 2 ? "#1e88e5" : "#fb8c00",
          }}
        />
      ))}
    </View>
  );
}
