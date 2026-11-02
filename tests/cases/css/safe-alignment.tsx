export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Boxes wider and taller than their containers: centered and at the end, past the start edge,
// and `safe`, which keeps them at the start; as flex items, as grid items, in a shorthand and
// with room, where `safe` changes nothing.
export default function Scene() {
  const row = { display: "flex", width: 60, height: 12, background: "#eceff1", marginBottom: 10 };
  const box = { width: 90, height: 8, flexShrink: 0, background: "#3949ab" };
  const tall = { width: 40, height: 24, background: "#e53935" };
  const grid = { display: "grid", width: 60, height: 12, background: "#eceff1" };
  return (
    <View style={{ padding: "8px 48px", gap: 16, display: "flex", flexDirection: "column" }}>
      <View style={{ ...row, justifyContent: "center" }}>
        <View style={box} />
      </View>
      <View style={{ ...row, justifyContent: "safe center" }}>
        <View style={box} />
      </View>
      <View style={{ ...row, justifyContent: "safe flex-end" }}>
        <View style={box} />
      </View>
      <View style={{ ...row, justifyContent: "unsafe flex-end" }}>
        <View style={box} />
      </View>
      <View style={{ ...row, alignItems: "safe center" }}>
        <View style={tall} />
      </View>
      <View style={{ ...grid, placeItems: "safe center" }}>
        <View style={{ ...box, width: 80 }} />
      </View>
      <View style={{ ...grid, justifyContent: "start" }}>
        <View style={{ ...box, width: 80, justifySelf: "safe end" }} />
      </View>
      <View style={{ ...row, justifyContent: "safe center" }}>
        <View style={{ ...box, width: 20 }} />
      </View>
    </View>
  );
}
