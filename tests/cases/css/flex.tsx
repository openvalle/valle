export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Flex layout: grow, shrink and basis, the flex shorthand, gaps, wrapping, and alignment on both
// axes.
export default function Scene() {
  const item = (background: string, style: object = {}) => (
    <View style={{ background, minWidth: 0, ...style }} />
  );
  const row = { display: "flex", height: 24, marginBottom: 4, background: "#e8eaf6" };
  return (
    <View style={{ padding: 4 }}>
      <View style={row}>
        {item("#3949ab", { flex: 1 })}
        {item("#5c6bc0", { flex: 2 })}
        {item("#7986cb", { width: 40 })}
      </View>
      <View style={{ ...row, gap: 6 }}>
        {item("#3949ab", { flexBasis: 100, flexShrink: 2 })}
        {item("#5c6bc0", { flexBasis: 100 })}
        {item("#7986cb", { flex: "1 1 50px" })}
        {item("#9fa8da", { flex: "none", width: 30 })}
      </View>
      <View style={{ ...row, justifyContent: "space-between", alignItems: "center" }}>
        {item("#00897b", { width: 30, height: 10 })}
        {item("#26a69a", { width: 30, height: 16 })}
        {item("#4db6ac", { width: 30, alignSelf: "flex-end", height: 6 })}
      </View>
      <View style={{ ...row, justifyContent: "space-evenly", alignItems: "flex-start" }}>
        {item("#00897b", { width: 20, height: 10 })}
        {item("#26a69a", { width: 20, height: 20 })}
        {item("#4db6ac", { width: 20 })}
      </View>
      <View style={{ ...row, justifyContent: "flex-end", columnGap: 3 }}>
        {item("#f9a825", { width: 24 })}
        {item("#fbc02d", { width: 24, marginLeft: "auto" })}
        {item("#fdd835", { width: 24, marginRight: 10 })}
      </View>
      <View
        style={{
          display: "flex",
          flexWrap: "wrap",
          height: 70,
          gap: "4px 8px",
          alignContent: "space-around",
          background: "#fce4ec",
        }}
      >
        {[50, 70, 40, 90, 30, 60, 80].map((width, i) => item("#d81b60", { width, height: 12 + i }))}
      </View>
      <View
        style={{
          display: "flex",
          flexDirection: "column",
          height: 60,
          marginTop: 4,
          gap: 2,
          background: "#e0f2f1",
        }}
      >
        {item("#00695c", { height: 10 })}
        {item("#00796b", { flexGrow: 1 })}
        {item("#00897b", { height: 12, width: "50%", alignSelf: "center" })}
      </View>
    </View>
  );
}
