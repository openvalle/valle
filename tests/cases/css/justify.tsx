export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// justify-items and justify-self outside grids: a block container's children, at their widths or
// as wide as their content.
export default function Scene() {
  return (
    <View style={{ justifyItems: "center", background: "#e8eaf6" }}>
      <View style={{ width: 40, height: 6, background: "#3949ab" }} />
      <View style={{ width: 30, height: 6, justifySelf: "end", background: "#7986cb" }} />
      <View style={{ height: 6, justifySelf: "start", background: "#3949ab" }} />
    </View>
  );
}
