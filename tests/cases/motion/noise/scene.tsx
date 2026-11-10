export const composition = { width: 280, height: 144, fps: 4, duration: 2.25 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 280, height: 144, background: "white" }}>
      {[0, 1, 2].map((row) =>
        Array.from({ length: 8 }, (_, i) => (
          <View
            key={`${row}-${i}`}
            style={{
              position: "absolute",
              left: 12 + i * 32,
              top: 14 + row * 44,
              width: 12,
              height: 12,
              background: row === 2 ? "#e66a38" : "#176bba",
              transform: `translateY(${20 * (row === 2 ? noise2d(17, seconds * 2, i) : noise1d(17 + i, seconds * 2))}px)`,
            }}
          />
        )),
      )}
    </View>
  );
}
