// error: errors/serif.tsx:5:10: font family serif is not built in yet
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <Text style={{ fontFamily: "serif" }}>Hi</Text>;
}
