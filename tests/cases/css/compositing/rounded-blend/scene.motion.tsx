export const composition = { width: 240, height: 200, fps: 1, duration: 1 };

export default function RoundedBlend() {
  return (
    <View style={{ width: 240, height: 200, backgroundColor: "white" }}>
      <View
        key="parent"
        style={{
          position: "absolute",
          left: 40,
          top: 20,
          width: 160,
          height: 160,
          backgroundColor: "aqua",
          isolation: "isolate",
        }}
      >
        <View
          key="clip"
          style={{
            width: 160,
            height: 160,
            borderRadius: 32,
            overflow: "hidden",
            mixBlendMode: "multiply",
          }}
        >
          <View style={{ width: 200, height: 200, backgroundColor: "yellow" }} />
        </View>
      </View>
    </View>
  );
}
