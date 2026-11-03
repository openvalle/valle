// error: errors/later-element.tsx:5:10: TypeError: <Glass> is not supported yet
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <Glass style={{ width: 32, height: 32 }} />;
}
