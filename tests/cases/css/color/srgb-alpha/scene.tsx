export const composition = { width: 360, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 60,
          height: 60,
          backgroundColor: "#ff0000",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 100,
          top: 20,
          width: 60,
          height: 60,
          backgroundColor: "rgb(0 128 0)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 60,
          height: 60,
          backgroundColor: "hsl(240 100% 50%)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 260,
          top: 20,
          width: 60,
          height: 60,
          backgroundColor: "color(srgb 0.2 0.4 0.6)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 100,
          width: 80,
          height: 60,
          backgroundColor: "rgb(0 0 0 / 50%)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 120,
          top: 100,
          width: 80,
          height: 60,
          backgroundColor: "rgb(255 0 0 / 25%)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 220,
          top: 100,
          width: 80,
          height: 60,
          backgroundColor: "transparent",
        }}
      />
    </View>
  );
}
