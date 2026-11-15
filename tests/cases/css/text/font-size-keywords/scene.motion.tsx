export const composition = { width: 440, height: 260, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 440, height: 260, backgroundColor: "white" }}>
      <View
        key="s0"
        style={{
          position: "absolute",
          left: 12,
          top: 12,
          fontSize: "xx-small",
          width: "4em",
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 220,
          top: 12,
          width: 36,
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        key="s1"
        style={{
          position: "absolute",
          left: 12,
          top: 42,
          fontSize: "x-small",
          width: "4em",
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 220,
          top: 42,
          width: 40,
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        key="s2"
        style={{
          position: "absolute",
          left: 12,
          top: 72,
          fontSize: "small",
          width: "4em",
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 220,
          top: 72,
          width: 52,
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        key="s3"
        style={{
          position: "absolute",
          left: 12,
          top: 102,
          fontSize: "medium",
          width: "4em",
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 220,
          top: 102,
          width: 64,
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        key="s4"
        style={{
          position: "absolute",
          left: 12,
          top: 132,
          fontSize: "large",
          width: "4em",
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 220,
          top: 132,
          width: 72,
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        key="s5"
        style={{
          position: "absolute",
          left: 12,
          top: 162,
          fontSize: "x-large",
          width: "4em",
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 220,
          top: 162,
          width: 96,
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        key="s6"
        style={{
          position: "absolute",
          left: 12,
          top: 192,
          fontSize: "xx-large",
          width: "4em",
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 220,
          top: 192,
          width: 128,
          height: 16,
          backgroundColor: "#2878b8",
        }}
      />
      <View style={{ position: "absolute", left: 12, top: 226, fontSize: 30 }}>
        <View
          key="relative"
          style={{
            font: "smaller/1 sans-serif",
            width: "4em",
            height: 16,
            backgroundColor: "#2878b8",
          }}
        />
      </View>
    </View>
  );
}
