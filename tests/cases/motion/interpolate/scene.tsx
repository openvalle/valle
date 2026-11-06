export const composition = { width: 256, height: 128, fps: 4, duration: 1.25 };

export default function Scene() {
  const { seconds } = useCurrentTime();
  const clamp = interpolate(seconds, [0.25, 0.75], [64, 160]);
  const extend = interpolate(seconds, [0.25, 0.75], [64, 160], {
    extrapolateLeft: "extend",
    extrapolateRight: "extend",
  });
  const length = interpolate(seconds, [0.25, 0.75], ["64px", "160px"]);
  return (
    <View key="canvas" style={{ width: 256, height: 128, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: clamp,
          top: 16,
          width: 16,
          height: 16,
          backgroundColor: "#176bba",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: extend,
          top: 48,
          width: 16,
          height: 16,
          backgroundColor: "#16a085",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: length,
          top: 80,
          width: 16,
          height: 16,
          backgroundColor: "#e66a38",
        }}
      />
      {seconds >= 0.5 && (
        <View
          style={{
            position: "absolute",
            left: 8,
            top: 112,
            width: 240,
            height: 4,
            backgroundColor: "#263238",
          }}
        />
      )}
    </View>
  );
}
