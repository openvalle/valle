export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Opacity and blend classes: opacities of the scale and written, on a child and over a stripe;
// each blend mode of a child, over a gradient; background layers blended with each other; and an
// isolated group, whose child blends with nothing under it.
export default function Scene() {
  const modes = [
    "mix-blend-multiply",
    "mix-blend-screen",
    "mix-blend-overlay",
    "mix-blend-darken",
    "mix-blend-lighten",
    "mix-blend-color-dodge",
    "mix-blend-color-burn",
    "mix-blend-hard-light",
    "mix-blend-soft-light",
    "mix-blend-difference",
    "mix-blend-exclusion",
    "mix-blend-hue",
    "mix-blend-saturation",
    "mix-blend-color",
    "mix-blend-luminosity",
    "mix-blend-plus-lighter",
  ];
  const backgrounds = ["bg-blend-multiply", "bg-blend-screen", "bg-blend-difference"];
  return (
    <View className="flex flex-wrap items-start gap-2 p-2 bg-neutral-100">
      <View className="flex gap-1 p-1 bg-linear-to-r from-yellow-300 to-teal-400">
        <View className="size-6 bg-rose-600 opacity-25" />
        <View className="size-6 bg-rose-600 opacity-50" />
        <View className="size-6 bg-rose-600 opacity-75" />
        <View className="size-6 bg-rose-600 opacity-[0.1]" />
        <View className="size-6 bg-rose-600 opacity-12.5" />
      </View>
      <View className="flex flex-wrap w-72 bg-linear-to-r from-red-500 via-green-400 to-blue-600">
        {modes.map((m) => (
          <View key={m} className={`w-9 h-7 bg-amber-300 ${m}`} />
        ))}
      </View>
      {backgrounds.map((b) => (
        <View
          key={b}
          className={`size-12 bg-linear-to-br from-sky-300 to-fuchsia-600 bg-yellow-400 ${b}`}
        />
      ))}
      <View className="flex p-1 bg-cyan-500">
        <View className="size-8 bg-rose-400 mix-blend-multiply" />
        <View className="isolate size-8">
          <View className="size-8 bg-rose-400 mix-blend-multiply" />
        </View>
        <View className="isolation-auto size-8">
          <View className="size-8 bg-rose-400 mix-blend-multiply" />
        </View>
      </View>
    </View>
  );
}
