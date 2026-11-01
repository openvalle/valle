// error: errors/permanent-style.tsx:5:10: style float is not supported: there is no document flow; use flex or grid layout
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <View style={{ float: "left" }} />;
}
