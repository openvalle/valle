export const composition = { width: 320, height: 128, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View
      style={{
        position: "relative",
        width: 320,
        height: 128,
        fontSize: 125,
        backgroundColor: "white",
      }}
    >
      <View
        key="bar0"
        style={{
          position: "absolute",
          left: 12,
          top: 12,
          height: 12,
          backgroundColor: "#17365d",
          width: "1ex",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 172,
          top: 12,
          width: 67,
          height: 12,
          backgroundColor: "#17365d",
        }}
      />
      <View
        key="bar1"
        style={{
          position: "absolute",
          left: 12,
          top: 40,
          height: 12,
          backgroundColor: "#17365d",
          width: "calc(2ex - 4px)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 172,
          top: 40,
          width: 130,
          height: 12,
          backgroundColor: "#17365d",
        }}
      />
      <View
        key="bar2"
        style={{
          position: "absolute",
          left: 12,
          top: 68,
          height: 12,
          backgroundColor: "#17365d",
          fontSize: "1ex",
          width: "1em",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 172,
          top: 68,
          width: 67,
          height: 12,
          backgroundColor: "#17365d",
        }}
      />
      <View
        key="bar3"
        style={{
          position: "absolute",
          left: 12,
          top: 96,
          height: 12,
          backgroundColor: "#17365d",
          width: "1ex",
          fontWeight: 700,
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 172,
          top: 96,
          width: 68.25,
          height: 12,
          backgroundColor: "#17365d",
        }}
      />
    </View>
  );
}
