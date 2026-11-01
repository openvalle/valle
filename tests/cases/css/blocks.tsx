export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Block flow: auto and fixed widths, percentages, auto margins, padding, min and max sizes,
// collapsing margins and display: none.
export default function Scene() {
  return (
    <View style={{ padding: 8, background: "#eceff1" }}>
      <View style={{ height: 12, background: "#90a4ae" }} />
      <View style={{ width: "50%", height: 12, marginTop: 6, background: "#78909c" }} />
      <View style={{ width: 100, height: 12, margin: "6px auto", background: "#607d8b" }} />
      <View style={{ width: 80, height: 12, marginLeft: "auto", background: "#546e7a" }} />
      <View
        style={{
          marginTop: 10,
          marginBottom: 4,
          padding: "4px 10% 6px 12px",
          background: "#b0bec5",
        }}
      >
        <View style={{ height: 10, marginTop: 8, background: "#455a64" }} />
        <View style={{ height: 10, marginTop: 6, marginBottom: 12, background: "#37474f" }} />
      </View>
      <View style={{ marginTop: 16 }}>
        <View style={{ height: 10, marginTop: 20, background: "#263238" }} />
      </View>
      <View style={{ display: "none", height: 40, background: "red" }}>
        <View style={{ height: 10 }} />
      </View>
      <View style={{ width: 300, maxWidth: "75%", minHeight: 8, background: "#ff7043" }} />
      <View
        style={{
          width: 10,
          minWidth: 60,
          height: 8,
          maxHeight: 6,
          marginTop: 4,
          background: "#f4511e",
        }}
      />
      <View style={{ height: "10%", marginTop: 4, background: "#bf360c" }} />
    </View>
  );
}
