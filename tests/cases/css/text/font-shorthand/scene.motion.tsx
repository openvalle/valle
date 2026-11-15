export const composition = { width: 320, height: 140, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 320, height: 140, padding: 16, backgroundColor: "white" }}>
      <Text key="bold" style={{ font: "700 24px/40px sans-serif", color: "#182433" }}>
        Valle 2026
      </Text>
      <Text
        key="reset"
        style={{
          fontWeight: 900,
          fontStyle: "italic",
          font: "20px/32px sans-serif",
          color: "#2563eb",
        }}
      >
        Font shorthand
      </Text>
    </View>
  );
}
