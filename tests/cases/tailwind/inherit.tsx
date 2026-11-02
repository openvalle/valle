export const composition = { width: 320, height: 120, fps: 1, duration: 1 };

// Colors as the parent has them, over classes Tailwind sorts before these: a text color and SVG
// paints.
export default function Scene() {
  return (
    <View className="p-2 fill-emerald-500 stroke-rose-500 text-indigo-600">
      <Text className="mb-2 text-sm text-amber-500 text-inherit">The parent's color</Text>
      <svg width={120} height={40} viewBox="0 0 30 10" className="fill-sky-500">
        <path d="M1 1H14V9H1Z" className="fill-amber-500 fill-inherit stroke-1" />
        <path d="M16 1H29V9H16Z" className="stroke-amber-500 stroke-inherit stroke-1" />
      </svg>
    </View>
  );
}
