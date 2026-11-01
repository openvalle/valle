// error: errors/class-name.tsx:5:10: className is not supported yet
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <View className="flex" />;
}
