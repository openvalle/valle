export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Outlines: outside the border box and over the next box's background, rounded with the border
// box and grown by the offset, inside the box with a negative offset, on flex items, clipped by
// an ancestor but not by the box itself, under a positioned box, and in a layer.
export default function Scene() {
  const box = (background: string, style: object) => (
    <View style={{ width: 52, height: 28, background, ...style }} />
  );
  return (
    <View style={{ padding: 12, display: "flex", flexDirection: "column", gap: 12 }}>
      <View>
        {box("#e3f2fd", { outline: "4px solid #1565c0" })}
        {box("#90caf9", { marginLeft: 20 })}
      </View>
      <View style={{ display: "flex", gap: 16, alignItems: "flex-start" }}>
        {box("#fce4ec", {
          borderRadius: 10,
          outline: "3px solid #ad1457",
          outlineOffset: 4,
        })}
        {box("#f3e5f5", {
          borderRadius: "50% / 30%",
          outline: "2px solid #6a1b9a",
          outlineOffset: 2,
        })}
        {box("#ede7f6", {
          border: "3px solid #4527a0",
          outline: "2px solid #b39ddb",
          outlineOffset: -8,
        })}
      </View>
      <View
        style={{
          overflow: "hidden",
          height: 44,
          padding: 6,
          background: "#e8f5e9",
          display: "flex",
          gap: 10,
        }}
      >
        {box("#a5d6a7", { outline: "5px solid #2e7d32", outlineOffset: 2 })}
        <View style={{ overflow: "hidden", width: 52, height: 28, outline: "2px solid #1b5e20" }}>
          {box("#66bb6a", { width: 80 })}
        </View>
      </View>
      <View style={{ position: "relative", display: "flex", gap: 12 }}>
        {box("#fff3e0", { outline: "6px solid #ef6c00" })}
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 10,
            width: 30,
            height: 30,
            background: "#ffcc80",
          }}
        />
        {box("#ffe0b2", { opacity: 0.6, outline: "4px solid #e65100", outlineOffset: 1 })}
      </View>
    </View>
  );
}
