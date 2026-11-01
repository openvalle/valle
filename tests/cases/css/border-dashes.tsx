export const composition = { width: 256, height: 330, fps: 1, duration: 1 };

// Dashed borders 1, 2, 4 and 6 px wide, and dotted ones 1, 2, 4, 6 and 10 px, square dots under
// 4 px; rounded, over a background; in four colors, cut at the corners; dashed on top only;
// translucent; of two widths, rounded; and a dashed outline and a rounded dotted one.
export default function Scene() {
  const box = { width: 64, height: 40, borderColor: "#263238", borderStyle: "dashed" };
  const dots = { ...box, borderStyle: "dotted" };
  return (
    <View style={{ display: "flex", padding: 8, flexDirection: "row", flexWrap: "wrap", gap: 10 }}>
      <View style={{ ...box, borderWidth: 1 }} />
      <View style={{ ...box, borderWidth: 2 }} />
      <View style={{ ...box, borderWidth: 4 }} />
      <View style={{ ...box, borderWidth: 6 }} />
      <View style={{ ...dots, borderWidth: 1 }} />
      <View style={{ ...dots, borderWidth: 2 }} />
      <View style={{ ...dots, borderWidth: 4 }} />
      <View style={{ ...dots, borderWidth: 6 }} />
      <View style={{ ...dots, borderWidth: 10 }} />
      <View style={{ ...box, borderWidth: 4, borderRadius: 12, background: "#ffe082" }} />
      <View style={{ ...dots, borderWidth: 6, borderRadius: 20, background: "#e8eaf6" }} />
      <View style={{ ...box, borderWidth: 6, borderColor: "#e53935 #1e88e5 #43a047 #fdd835" }} />
      <View style={{ ...box, borderWidth: 5, borderStyle: "dashed solid solid solid" }} />
      <View
        style={{
          ...box,
          borderWidth: 8,
          borderColor: "rgb(57 73 171 / 50%)",
          background: "#e8eaf6",
        }}
      />
      <View style={{ ...dots, borderWidth: "4px 8px", borderRadius: 16 }} />
      <View
        style={{
          ...box,
          margin: 6,
          borderStyle: "none",
          background: "#e8eaf6",
          outline: "3px dashed #00897b",
          outlineOffset: 3,
        }}
      />
      <View
        style={{
          ...box,
          margin: 6,
          borderStyle: "none",
          background: "#e8eaf6",
          borderRadius: 10,
          outline: "5px dotted #8e24aa",
          outlineOffset: 1,
        }}
      />
    </View>
  );
}
