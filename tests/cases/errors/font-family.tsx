// error: errors/font-family.tsx:5:10: font family Brand: font resources are not supported yet
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <Text style={{ fontFamily: "Brand, sans-serif" }}>Hi</Text>;
}
