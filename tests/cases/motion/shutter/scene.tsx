export const composition = { width: 256, height: 128, fps: 30, duration: 0.4 };

function Bar({ moving, color }) {
  const { seconds } = useCurrentTime();
  return (
    <View
      style={{
        position: "absolute",
        left: moving ? 16 + 480 * seconds : 40,
        width: 20,
        height: 24,
        backgroundColor: color,
      }}
    />
  );
}

export default function Scene() {
  return (
    <View key="canvas" style={{ width: 256, height: 128, backgroundColor: "white" }}>
      <View style={{ position: "absolute", top: 8, width: 256, height: 24 }}>
        <Shutter samples={4} angle={180}>
          <Bar moving={true} color="blue" />
        </Shutter>
      </View>
      <View style={{ position: "absolute", top: 48, width: 256, height: 24 }}>
        <Shutter samples={4} angle={180}>
          <Bar moving={false} color="green" />
        </Shutter>
      </View>
      <View style={{ position: "absolute", top: 88, width: 256, height: 24 }}>
        <Shutter samples={4} angle={0}>
          <Bar moving={true} color="red" />
        </Shutter>
      </View>
    </View>
  );
}
