export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Background classes: linear gradients to a side, at an angle, negative, the long way round the
// hue and in sRGB, with a via stop and stop positions, and written; radial gradients, centered
// and at a written place; conic gradients, from the top and from an angle; and a written image
// sized, placed and repeated, with its box clipped and its origin moved, and text clipped to it.
export default function Scene() {
  const border = { border: "4px dashed #37474f" };
  return (
    <View className="flex flex-wrap items-start gap-2 p-2 bg-neutral-100">
      <View className="w-16 h-10 bg-linear-to-r from-sky-500 to-indigo-500" />
      <View className="w-16 h-10 bg-linear-to-br from-pink-500 via-yellow-300 to-emerald-500" />
      <View className="w-16 h-10 bg-linear-45 from-red-500 from-10% via-amber-400 via-30% to-transparent" />
      <View className="w-16 h-10 -bg-linear-30/longer from-red-500 to-blue-500" />
      <View className="w-16 h-10 bg-linear-to-r/srgb from-red-500 to-blue-500" />
      <View className="w-16 h-10 bg-linear-[25deg,#e53935_10%,#1e88e5]" />
      <View className="size-12 bg-radial from-white to-violet-600" />
      <View className="size-12 bg-radial-[at_25%_25%] from-yellow-200 to-orange-600 to-75%" />
      <View
        className="size-12 bg-conic from-red-500 via-green-500 to-blue-500"
        style={{ borderRadius: "50%" }}
      />
      <View className="size-12 bg-conic-180 from-indigo-600 via-sky-300 to-indigo-600" />
      <View
        className="w-20 h-12 bg-[linear-gradient(#43a047,#fdd835)] bg-size-[16px_16px] bg-no-repeat bg-center bg-sky-100"
        style={border}
      />
      <View
        className="w-20 h-12 p-2 bg-[radial-gradient(#e53935_40%,transparent_45%)] bg-size-[12px_12px] bg-repeat-x bg-top-left bg-origin-content bg-clip-padding bg-amber-100"
        style={border}
      />
      <View
        className="w-20 h-12 p-1 bg-linear-to-b from-teal-400 to-blue-700 bg-size-[100%_50%] bg-repeat-space bg-clip-content"
        style={border}
      />
      <Text className="text-2xl font-black bg-linear-to-r from-fuchsia-600 to-orange-500 bg-clip-text text-transparent">
        Clipped
      </Text>
    </View>
  );
}
