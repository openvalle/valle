export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Shadow classes: box shadows of the theme, in a color, at an opacity and written, one of them
// inset; inset shadows of the theme, and written in a color; rings of widths and colors, outside
// offsets of the background's color and of their own, inset, and inset rings; and text shadows of
// the theme, in a color, and written.
export default function Scene() {
  const box = "size-8 rounded-md bg-white";
  const theme = [
    "shadow-2xs",
    "shadow-xs",
    "shadow-sm",
    "shadow-md",
    "shadow-lg",
    "shadow-xl",
    "shadow-2xl",
  ];
  return (
    <View className="flex flex-wrap items-start gap-3 p-3 bg-neutral-100">
      {theme.map((s) => (
        <View key={s} className={`${box} ${s}`} />
      ))}
      <View className={`${box} shadow-lg shadow-indigo-500/50`} />
      <View className={`${box} shadow-xl/30`} />
      <View className={`${box} shadow-[0_0_12px_#e91e63]`} />
      <View className={`${box} shadow-[inset_0_2px_4px_#0006,0_4px_8px_#1e88e5]`} />
      <View className={`${box} inset-shadow-sm`} />
      <View className={`${box} inset-shadow-[0_3px_6px] inset-shadow-amber-600`} />
      <View className={`${box} ring-2 ring-sky-500`} />
      <View className={`${box} ring-4 ring-rose-500 ring-offset-2`} />
      <View
        className={`${box} ring-2 ring-emerald-500 ring-offset-4 ring-offset-amber-200 shadow-md`}
      />
      <View className={`${box} ring-inset ring-4 ring-violet-500`} />
      <View className={`${box} inset-ring-2 inset-ring-cyan-500/60`} />
      <View className={`${box} ring-[3px] ring-black/30`} />
      <Text className="text-2xl font-black text-rose-500 text-shadow-md">Shade</Text>
      <Text className="text-2xl font-black text-sky-100 text-shadow-lg text-shadow-sky-700">
        Sky
      </Text>
      <Text className="text-2xl font-black text-amber-300 text-shadow-[2px_2px_0_#000]">Pop</Text>
    </View>
  );
}
