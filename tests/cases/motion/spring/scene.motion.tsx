export const composition = { width: 280, height: 144, fps: 8, duration: 2.25 };
const variants = [
  { stiffness: 39.47841760435743, damping: 0, color: "#176bba" },
  { stiffness: 16, damping: 8, color: "#16a085" },
  { stiffness: 16, damping: 10, color: "#e66a38" },
];
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 280, height: 144, background: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 124,
          top: 8,
          width: 1,
          height: 128,
          background: "#bbb",
        }}
      />
      {variants.map((v, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: 24,
            top: 20 + i * 44,
            width: 14,
            height: 14,
            background: v.color,
            transform: `translateX(${100 * spring(seconds - 0.25, { stiffness: v.stiffness, damping: v.damping })}px)`,
          }}
        />
      ))}
    </View>
  );
}
