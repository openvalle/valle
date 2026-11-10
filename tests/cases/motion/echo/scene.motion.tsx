export const composition = { width: 256, height: 112, fps: 4, duration: 2 };

function Dot() {
  const { seconds } = useCurrentTime();
  return (
    <View
      style={{
        position: "absolute",
        left: 16 + 96 * seconds,
        top: 0,
        width: 32,
        height: 32,
        backgroundColor: seconds < 0.75 ? "#176bba" : "#e66a38",
      }}
    />
  );
}

export default function Scene() {
  return (
    <View key="canvas" style={{ width: 256, height: 112, backgroundColor: "white" }}>
      <View style={{ position: "absolute", top: 16, width: 256, height: 32 }}>
        <Echo count={2} delay={0.25} decay={0.5}>
          <Dot />
        </Echo>
      </View>
      <View style={{ position: "absolute", top: 64, width: 256, height: 32 }}>
        <Echo count={3} delay={0.125} decay={0.5}>
          <Dot />
        </Echo>
      </View>
    </View>
  );
}
