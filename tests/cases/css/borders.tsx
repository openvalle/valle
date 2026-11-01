export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Borders: sides in their own widths and colors meeting at the corners, rounded corners with a
// rounded or square inside, an elliptical border, a clip inside a border, text and flex items in
// bordered boxes, translucent sides over a background, square and rounded, and a border in a
// layer.
export default function Scene() {
  const label = (text: string) => <Text style={{ fontSize: 10 }}>{text}</Text>;
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      <View
        style={{
          width: 52,
          height: 36,
          borderStyle: "solid",
          borderWidth: "4px 8px 12px 2px",
          borderColor: "#e53935 #1e88e5 #43a047 #fdd835",
        }}
      />
      <View
        style={{
          width: 52,
          height: 36,
          border: "10px solid #8e24aa",
          borderTopColor: "#ce93d8",
          borderRightColor: "#4a148c",
          borderRadius: 16,
          background: "#f3e5f5",
        }}
      />
      <View
        style={{
          width: 52,
          height: 36,
          border: "12px solid #00897b",
          borderRadius: 6,
          background: "#e0f2f1",
        }}
      />
      <View
        style={{
          width: 52,
          height: 36,
          border: "3px solid #f4511e",
          borderRadius: "50%",
          background: "#fbe9e7",
        }}
      />
      <View
        style={{
          overflow: "hidden",
          width: 112,
          height: 48,
          border: "6px solid #3949ab",
          borderRadius: 18,
          background: "#e8eaf6",
        }}
      >
        <View style={{ height: 14, marginLeft: -10, width: 140, background: "#7986cb" }} />
        {label("Clipped inside the border, past it")}
      </View>
      <View
        style={{
          width: 112,
          display: "flex",
          gap: 4,
          padding: 4,
          border: "2px solid #6d4c41",
          borderInlineStartWidth: 8,
        }}
      >
        <View style={{ flexGrow: 1, borderBottom: "4px solid #a1887f" }}>{label("Flex")}</View>
        <View style={{ flexGrow: 1, borderBlock: "thin solid #4e342e" }}>{label("Items")}</View>
      </View>
      <View
        style={{
          width: 52,
          height: 36,
          background: "#ffeb3b",
          border: "8px solid rgb(0 0 255 / 40%)",
          borderLeftColor: "transparent",
        }}
      />
      <View
        style={{
          width: 52,
          height: 36,
          opacity: 0.5,
          border: "5px solid #c62828",
          borderRadius: "20px 4px",
        }}
      />
      <View style={{ width: 52, height: 36, color: "#2e7d32", border: "thick solid" }}>
        {label("Color")}
      </View>
      <View style={{ width: 52, height: 36, borderStyle: "solid none", borderWidth: 6 }} />
      <View
        style={{
          width: 52,
          height: 36,
          background: "#ffeb3b",
          border: "6px solid rgb(0 0 255 / 40%)",
          borderRadius: 14,
        }}
      />
    </View>
  );
}
