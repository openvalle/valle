export const composition = { width: 320, height: 64, fps: 1, duration: 1 };

// A background of the text color, inherited and the box's own.
export default function Scene() {
  return (
    <View className="p-2 text-indigo-600">
      <View className="mb-2 h-4 w-20 bg-current" />
      <View className="h-4 w-20 bg-current text-teal-500" />
    </View>
  );
}
