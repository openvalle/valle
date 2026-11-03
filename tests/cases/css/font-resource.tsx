export const controls = {
  resources: {
    display: resource({ kind: "font", default: "../resources/Bungee-Regular.ttf" }),
  },
};

export const composition = { width: 256, height: 160, fps: 1, duration: 1 };

// A font resource is a family by its name: a heading in it, a Span of it in sans-serif text, and
// a list that takes it first, in any ASCII case.
export default function Scene() {
  return (
    <View style={{ padding: 8, gap: 6, color: "#263238" }}>
      <Text style={{ fontFamily: "display", fontSize: 24, color: "#e53935" }}>Valle 2026</Text>
      <Text style={{ fontSize: 16 }}>
        Plain and <Span style={{ fontFamily: "display", color: "#1e88e5" }}>display</Span> text
      </Text>
      <Text style={{ fontFamily: "Display, serif", fontSize: 18, background: "#eceff1" }}>
        First of a list
      </Text>
    </View>
  );
}
