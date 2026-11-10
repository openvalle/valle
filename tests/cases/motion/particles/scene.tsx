export const composition = { width: 280, height: 144, fps: 8, duration: 2.25 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 280, height: 144, background: "white" }}>
      {[0, 1, 2].map((row) =>
        particles(seconds, {
          seed: 17,
          count: 4,
          birth: { interval: 0.25 },
          lifetime: 0.75,
          loop: row !== 0,
          emitter: { x: 24, y: 12 + row * 44, width: row === 0 ? 0 : 12, height: 0 },
          velocity: { x: row === 0 ? [64, 64] : [50, 80], y: [0, 0] },
          gravity: { x: 0, y: 32 },
          forces: row === 2 ? [drag(1), curlNoise({ seed: 9, scale: 0.03, strength: 8 })] : [],
        }).map(
          (p) =>
            p.alive && (
              <View
                key={`${row}-${p.key}`}
                style={{
                  position: "absolute",
                  left: 0,
                  top: 0,
                  width: 8,
                  height: 8,
                  background: row === 0 ? "#176bba" : row === 1 ? "#16a085" : "#e66a38",
                  transform: `translate(${p.x}px, ${p.y}px)`,
                }}
              />
            ),
        ),
      )}
    </View>
  );
}
