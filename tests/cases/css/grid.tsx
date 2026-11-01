export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Grid layout: named lines and spans, auto-fill tracks, item alignment in grid areas, dense
// packing with order, and tracks sized by text with an absolute box in a grid area.
export default function Scene() {
  const label = (text: string) => <Text style={{ fontSize: 10 }}>{text}</Text>;
  const cell = (background: string, style: object = {}) => (
    <View style={{ background, ...style }} />
  );
  return (
    <View style={{ padding: 8 }}>
      <View
        style={{
          display: "grid",
          gridTemplateColumns: "[side] 48px [main] 1fr 1fr [end]",
          gridTemplateRows: "16px auto",
          gap: 4,
          marginBottom: 6,
        }}
      >
        {cell("#3949ab", { gridColumn: "side / end" })}
        <View style={{ gridRow: "2 / span 2", background: "#5c6bc0" }}>{label("Side")}</View>
        <View style={{ background: "#7986cb" }}>{label("One")}</View>
        <View style={{ background: "#9fa8da" }}>{label("Two")}</View>
        <View style={{ gridColumn: "main / end", background: "#c5cae9" }}>{label("Wide")}</View>
      </View>
      <View
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(50px, 1fr))",
          gap: 4,
          marginBottom: 6,
        }}
      >
        {[1, 2, 3, 4, 5, 6].map((n) => (
          <View style={{ padding: 2, background: n % 2 ? "#26a69a" : "#80cbc4" }}>
            {label(`Card ${n}`)}
          </View>
        ))}
      </View>
      <View
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(3, 1fr)",
          gridAutoRows: 24,
          gap: 3,
          justifyItems: "center",
          alignItems: "end",
          marginBottom: 6,
          background: "#fff3e0",
        }}
      >
        <View style={{ background: "#fb8c00" }}>{label("Centre")}</View>
        <View style={{ justifySelf: "stretch", alignSelf: "start", background: "#ffa726" }}>
          {label("Stretch")}
        </View>
        {cell("#ef6c00", { justifySelf: "end", width: 20, height: 10 })}
      </View>
      <View
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(4, 1fr)",
          gridAutoRows: 10,
          gridAutoFlow: "row dense",
          gap: 2,
          marginBottom: 6,
        }}
      >
        {cell("#8e24aa", { gridColumn: "span 3" })}
        {cell("#ab47bc", { gridColumn: "span 2" })}
        {cell("#ce93d8")}
        {cell("#4a148c", { order: -1 })}
      </View>
      <View
        style={{
          position: "relative",
          display: "grid",
          gridTemplateColumns: "min-content fit-content(60px) 1fr",
          gap: 4,
          background: "#e8f5e9",
        }}
      >
        <Text style={{ fontSize: 10, background: "#a5d6a7" }}>Min content</Text>
        <Text style={{ fontSize: 10, background: "#81c784" }}>Fit to sixty pixels</Text>
        <View style={{ display: "inline-block", background: "#66bb6a" }}>{label("Rest")}</View>
        {cell("#1b5e20", {
          position: "absolute",
          gridColumn: "2 / 4",
          gridRow: "1",
          left: 0,
          right: 0,
          bottom: 0,
          height: 3,
        })}
      </View>
    </View>
  );
}
