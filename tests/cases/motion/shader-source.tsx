export const composition = { width: 160, height: 96, fps: 4, duration: 1 };

export const controls = {
  resources: {
    wave: resource({ kind: "shader", default: "../resources/wave.glsl" }),
  },
};

// A shader read from a file, with the colors and the time it is given.
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ padding: 16 }}>
      <ShaderLayer
        source="resource://wave"
        uniforms={{ time: seconds * 6, low: "#00897b", high: "#ffeb3b" }}
        style={{ width: 128, height: 64, borderRadius: 12 }}
      />
    </View>
  );
}
