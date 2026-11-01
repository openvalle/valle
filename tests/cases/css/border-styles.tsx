export const composition = { width: 256, height: 200, fps: 1, duration: 1 };

// Border styles: double lines 3, 4, 7 and 10 px wide; rounded, over a background; in four
// colors; of four widths; double on top only; translucent; and a rounded double outline.
export default function Scene() {
  const box = { width: 64, height: 40, borderColor: "#263238", borderStyle: "double" };
  return (
    <View style={{ display: "flex", padding: 8, flexDirection: "row", flexWrap: "wrap", gap: 10 }}>
      <View style={{ ...box, borderWidth: 3 }} />
      <View style={{ ...box, borderWidth: 4 }} />
      <View style={{ ...box, borderWidth: 7 }} />
      <View style={{ ...box, borderWidth: 10 }} />
      <View style={{ ...box, borderWidth: 9, borderRadius: 14, background: "#ffe082" }} />
      <View
        style={{
          ...box,
          borderWidth: 8,
          borderColor: "#e53935 #1e88e5 #43a047 #fdd835",
        }}
      />
      <View style={{ ...box, borderWidth: "9px 3px 6px 12px", borderRadius: 10 }} />
      <View style={{ ...box, borderWidth: 8, borderStyle: "double solid solid solid" }} />
      <View
        style={{
          ...box,
          borderWidth: 9,
          borderColor: "rgb(57 73 171 / 50%)",
          background: "#e8eaf6",
        }}
      />
      <View
        style={{
          ...box,
          margin: 10,
          borderStyle: "none",
          background: "#e8eaf6",
          borderRadius: 8,
          outline: "7px double #00897b",
          outlineOffset: 3,
        }}
      />
    </View>
  );
}
