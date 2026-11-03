export const composition = { width: 320, height: 128, fps: 4, duration: 1 };

// Letters that rise into place one after another, turning as they come, and words that come
// into focus in turn, the last in another color.
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 320, height: 128, padding: 12, gap: 12, background: "#fff3e0" }}>
      <Text
        split="char"
        perUnit={(u) => {
          const shown = clamp(seconds * 4 - u.index * 0.3, 0, 1);
          return {
            opacity: shown,
            translate: `0 ${(1 - shown) * 24}px`,
            rotate: `${(1 - shown) * -30}deg`,
          };
        }}
        style={{ fontSize: 40, fontWeight: 800, color: "#d84315" }}
      >
        Rise up
      </Text>
      <Text
        split="word"
        perUnit={(u) => ({
          filter: `blur(${clamp(4 - seconds * 8 + u.index * 2, 0, 4)}px)`,
          color: u.index === u.count - 1 ? "#6a1b9a" : "#37474f",
        })}
        style={{ fontSize: 24 }}
      >
        words come into focus
      </Text>
    </View>
  );
}
