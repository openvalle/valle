export const composition = { width: 360, height: 140, fps: 1, duration: 1 };

export default function PreservedSpaces() {
  return (
    <View style={{ width: 360, height: 140, backgroundColor: "white" }}>
      <Text
        key="text"
        style={{
          position: "absolute",
          left: 20,
          top: 24,
          width: 320,
          height: 88,
          boxSizing: "border-box",
          padding: 8,
          border: "2px solid #ccd6e0",
          backgroundColor: "#f4f7fa",
          color: "#17324d",
          fontFamily: "sans-serif",
          fontSize: 24,
          lineHeight: "32px",
          whiteSpace: "pre",
        }}
      >
        {"Valle  motion\nFrame  by  frame"}
      </Text>
    </View>
  );
}
