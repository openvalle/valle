// error: errors/later-function.tsx:5: TypeError: fitText() is not supported yet…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const size = fitText("Valle", { width: 64 });
  return <View style={{ width: size }} />;
}
