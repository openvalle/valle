export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Sizes from content: the min-content, max-content and fit-content widths of text, fit-content in
// a narrow box, stretch, flex-basis: content, and an absolute box as wide as its text.
export default function Scene() {
  const label = (text: string) => <Text style={{ fontSize: 10 }}>{text}</Text>;
  const box = (width: string, background: string) => (
    <View style={{ width, background, marginBottom: 4 }}>{label("Sized by text")}</View>
  );
  return (
    <View style={{ padding: 8 }}>
      {box("min-content", "#c5cae9")}
      {box("max-content", "#9fa8da")}
      {box("fit-content", "#7986cb")}
      <View style={{ width: 40 }}>{box("fit-content", "#5c6bc0")}</View>
      {box("stretch", "#3949ab")}
      <View style={{ display: "flex", height: 24, background: "#e8eaf6" }}>
        <View style={{ flexBasis: "content", background: "#c5cae9" }}>{label("Basis")}</View>
        <View style={{ width: "max-content", background: "#9fa8da" }}>{label("Max content")}</View>
        <View style={{ height: "min-content", background: "#7986cb" }}>{label("Min")}</View>
      </View>
      <View style={{ position: "relative", height: 24 }}>
        <View style={{ position: "absolute", top: 4, width: "max-content", background: "#5c6bc0" }}>
          {label("Absolute max-content")}
        </View>
      </View>
    </View>
  );
}
