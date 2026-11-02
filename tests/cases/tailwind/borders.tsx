export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Border classes: widths of the scale, in px and written, on each side and the logical ones;
// colors at an opacity; dashed, dotted and double styles; radii of the scale, full, none, written
// and per corner; lines between children, reversed and styled; and outlines with their width,
// style, color and offset.
export default function Scene() {
  return (
    <View className="flex flex-wrap items-start gap-3 p-3 bg-slate-50">
      <View className="size-10 border border-slate-900 bg-white" />
      <View className="size-10 border-4 border-sky-500/60 rounded-lg bg-white" />
      <View className="size-10 border-x-8 border-y-2 border-x-rose-500 border-y-indigo-500" />
      <View className="size-10 border-t-4 border-b-[6px] border-l-2 border-r-0 border-emerald-600" />
      <View className="size-10 border-s-4 border-e-2 border-amber-500 border-dashed" />
      <View className="size-10 border-4 border-dotted border-violet-600 rounded-xl" />
      <View className="size-10 border-[6px] border-double border-teal-700" />
      <View className="size-10 rounded-full bg-pink-500 border-2 border-white outline-2 outline-pink-500" />
      <View className="w-16 h-10 rounded-t-2xl rounded-br-[18px] bg-cyan-600" />
      <View className="w-16 h-10 rounded-none rounded-ss-full rounded-ee-full bg-orange-500" />
      <View className="flex w-24 h-8 divide-x-2 divide-slate-400 bg-white">
        <View className="flex-1" />
        <View className="flex-1" />
        <View className="flex-1" />
      </View>
      <View className="flex flex-col w-16 divide-y-4 divide-y-reverse divide-dashed divide-red-500/70 bg-white">
        <View className="h-3" />
        <View className="h-3" />
        <View className="h-3" />
      </View>
      <View className="size-10 bg-lime-300 outline-4 outline-dashed outline-offset-2 outline-lime-700" />
      <View className="size-10 bg-sky-200 outline outline-dotted -outline-offset-4 outline-sky-700" />
      <View className="size-10 rounded-md bg-fuchsia-300 outline-[3px] outline-double outline-[#4a148c]" />
    </View>
  );
}
