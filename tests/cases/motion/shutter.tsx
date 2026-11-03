export const composition = { width: 128, height: 128, fps: 4, duration: 1 };

// A bar that turns a full circle a second, blurred over half of each frame: 45°.
function Bar() {
  const { seconds } = useCurrentTime();
  return (
    <View
      style={{
        position: "absolute",
        left: 16,
        top: 58,
        width: 96,
        height: 12,
        borderRadius: 6,
        background: "#8e24aa",
        rotate: `${seconds * 360}deg`,
      }}
    />
  );
}

export default function Scene() {
  return (
    <Shutter samples={16} angle={180}>
      <Bar />
    </Shutter>
  );
}
