export const composition = { width: 240, height: 160, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ position: "relative", width: 240, height: 160, backgroundColor: "white" }}>
      <View
        key="bar0"
        style={{
          position: "absolute",
          left: 12,
          top: 12,
          width: "1in",
          height: "9pt",
          backgroundColor: "#17365d",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 132,
          top: 12,
          width: 96,
          height: 12,
          backgroundColor: "#17365d",
        }}
      />
      <View
        key="bar1"
        style={{
          position: "absolute",
          left: 12,
          top: 36,
          width: "2.54cm",
          height: "9pt",
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 132,
          top: 36,
          width: 96,
          height: 12,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        key="bar2"
        style={{
          position: "absolute",
          left: 12,
          top: 60,
          width: "25.4mm",
          height: "9pt",
          backgroundColor: "#d06030",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 132,
          top: 60,
          width: 96,
          height: 12,
          backgroundColor: "#d06030",
        }}
      />
      <View
        key="bar3"
        style={{
          position: "absolute",
          left: 12,
          top: 84,
          width: "72pt",
          height: "9pt",
          backgroundColor: "#31906b",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 132,
          top: 84,
          width: 96,
          height: 12,
          backgroundColor: "#31906b",
        }}
      />
      <View
        key="bar4"
        style={{
          position: "absolute",
          left: 12,
          top: 108,
          width: "6pc",
          height: "9pt",
          backgroundColor: "#8050a0",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 132,
          top: 108,
          width: 96,
          height: 12,
          backgroundColor: "#8050a0",
        }}
      />
      <View
        key="bar5"
        style={{
          position: "absolute",
          left: 12,
          top: 132,
          width: "calc(1in - 36pt)",
          height: "9pt",
          backgroundColor: "#17365d",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 132,
          top: 132,
          width: 48,
          height: 12,
          backgroundColor: "#17365d",
        }}
      />
    </View>
  );
}
