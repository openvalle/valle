export const composition = { width: 200, height: 140, fps: 1, duration: 1 };

export default function ContentBox() {
  return (
    <View style={{ width: 200, height: 140, backgroundColor: "white" }}>
      <View
        key="box"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          boxSizing: "content-box",
          width: 100,
          height: 60,
          padding: 10,
          border: "4px solid black",
          backgroundColor: "#00aa00",
        }}
      />
    </View>
  );
}
