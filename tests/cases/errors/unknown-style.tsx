// error: errors/unknown-style.tsx:5:10: style colour is not supported
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <View style={{ colour: "red" }} />;
}
