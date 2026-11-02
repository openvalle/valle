// error: errors/tailwind-variant.tsx:5:10: className hover:p-4: variants are not supported: …
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <View className="block hover:p-4" />;
}
