export const composition = { width: 256, height: 360, fps: 1, duration: 1 };

// Border styles: double lines 3, 4, 7 and 10 px wide; rounded, over a background; in four
// colors; of four widths; double on top only; translucent; and a rounded double outline. Groove
// and an odd ridge; inset in black, whose shades are both lighter; outset, rounded; a rounded
// ridge over a background; a groove of four widths; inset translucent; and an outset outline.
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
      <View style={{ ...box, border: "8px groove #4caf50" }} />
      <View style={{ ...box, border: "7px ridge #4caf50" }} />
      <View style={{ ...box, border: "8px inset #000" }} />
      <View
        style={{ ...box, border: "8px outset #e53935", borderRadius: 12, background: "#e8eaf6" }}
      />
      <View
        style={{ ...box, border: "12px ridge #1e88e5", borderRadius: 20, background: "#ffe082" }}
      />
      <View style={{ ...box, border: "groove #8e24aa", borderWidth: "4px 10px 7px 12px" }} />
      <View style={{ ...box, border: "9px inset rgb(76 175 80 / 50%)", background: "#e8eaf6" }} />
      <View
        style={{
          ...box,
          margin: 8,
          borderStyle: "none",
          background: "#e8eaf6",
          outline: "6px outset #fdd835",
          outlineOffset: 2,
        }}
      />
    </View>
  );
}
