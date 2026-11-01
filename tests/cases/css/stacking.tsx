export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Stacking: z-index on positioned boxes, below and above in-flow content, in a stacking context
// and out of one, on flex and grid items, with opacity, ignored on a block with opacity, and in a
// fixed box.
export default function Scene() {
  const card = (background: string, style: object) => (
    <View style={{ position: "relative", width: 40, height: 24, background, ...style }} />
  );
  return (
    <View style={{ padding: 8 }}>
      <View style={{ position: "relative", height: 40, marginBottom: 8 }}>
        {card("#1565c0", { position: "absolute", left: 0, top: 0, zIndex: 3 })}
        {card("#42a5f5", { position: "absolute", left: 20, top: 8, zIndex: 1 })}
        {card("#90caf9", { position: "absolute", left: 40, top: 16, zIndex: 2 })}
        {card("#0d47a1", { position: "absolute", left: 60, top: 4 })}
      </View>
      <View style={{ display: "flex", gap: 16, marginBottom: 8 }}>
        <View style={{ width: 80, height: 30, background: "#c8e6c9" }}>
          {card("#2e7d32", { zIndex: -1, left: 20, top: 10 })}
        </View>
        <View
          style={{ position: "relative", zIndex: 0, width: 80, height: 30, background: "#c8e6c9" }}
        >
          {card("#2e7d32", { zIndex: -1, left: 20, top: 10 })}
          <View style={{ width: 30, height: 12, background: "#66bb6a" }} />
        </View>
      </View>
      <View style={{ display: "flex", height: 24, marginBottom: 8 }}>
        <View style={{ width: 50, background: "#ef6c00", zIndex: 2 }} />
        <View style={{ width: 50, marginLeft: -20, background: "#ffa726" }} />
        <View style={{ width: 50, marginLeft: -20, background: "#ffcc80", zIndex: 1 }} />
      </View>
      <View style={{ display: "grid", gridTemplateColumns: "60px", marginBottom: 8 }}>
        <View style={{ gridArea: "1 / 1", height: 20, background: "#8e24aa", zIndex: 1 }} />
        <View style={{ gridArea: "1 / 1", height: 20, marginLeft: 20, background: "#ce93d8" }} />
      </View>
      <View style={{ position: "relative", height: 30 }}>
        <View
          style={{
            opacity: 0.7,
            zIndex: 1,
            position: "relative",
            width: 60,
            height: 24,
            background: "#00897b",
          }}
        />
        {card("#004d40", { position: "absolute", left: 30, top: 6 })}
      </View>
      <View style={{ width: 100, background: "#e1f5fe" }}>
        <View style={{ height: 20, background: "#0288d1" }} />
        <View
          style={{
            opacity: 0.8,
            zIndex: -1,
            width: 60,
            height: 20,
            marginTop: -10,
            marginLeft: 20,
            background: "#ff7043",
          }}
        />
      </View>
      <View
        style={{
          position: "fixed",
          right: 8,
          bottom: 8,
          width: 40,
          height: 40,
          background: "#f9a825",
        }}
      >
        {card("#f57f17", { zIndex: -1, left: -10, top: -10, width: 30, height: 30 })}
      </View>
    </View>
  );
}
