export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Transform classes: translations of the scale, of fractions, full, negative and written; scales
// of both axes, of one, negative and written; rotations of the theme, negative and written, about
// origins; skews; a transform written; and in 3D, turned about each axis under perspectives of
// the theme and written, moved in depth, in a kept 3D space and with their backs hidden.
export default function Scene() {
  const box = "size-8 bg-sky-500 border-2 border-indigo-800";
  return (
    <View className="flex flex-wrap items-start gap-3 p-3 bg-neutral-100">
      <View className="flex gap-7 bg-amber-100">
        <View className={`${box} translate-x-2 translate-y-1`} />
        <View className={`${box} -translate-x-1/2`} />
        <View className={`${box} translate-y-full`} />
        <View className={`${box} translate-[3px]`} />
        <View className={`${box} -translate-x-[10%] translate-y-px`} />
      </View>
      <View className="flex gap-3 pt-4">
        <View className={`${box} scale-75`} />
        <View className={`${box} scale-x-50`} />
        <View className={`${box} -scale-y-125`} />
        <View className={`${box} scale-[1.2]`} />
        <View className={`${box} rotate-45`} />
        <View className={`${box} -rotate-12 origin-top-left`} />
        <View className={`${box} rotate-[30deg] origin-[10%_20%]`} />
      </View>
      <View className="flex gap-3">
        <View className={`${box} skew-x-12`} />
        <View className={`${box} -skew-y-6`} />
        <View className={`${box} skew-6`} />
        <View className={`${box} transform-[rotate(10deg)_scale(0.8)]`} />
        <View className={`${box} rotate-z-30 skew-x-6`} />
        <View className={`${box} transform`} />
      </View>
      <View className="flex gap-3">
        <View className="perspective-near">
          <View className={`${box} rotate-x-45 rotate-z-12`} />
        </View>
        <View className="perspective-[200px] perspective-origin-top-left">
          <View className={`${box} rotate-y-45 rotate-z-12`} />
        </View>
        <View className="perspective-dramatic">
          <View className="size-4 bg-sky-500 translate-z-12.5" />
        </View>
        <View className="perspective-normal">
          <View className={`${box} transform-3d rotate-y-30`}>
            <View className="size-4 bg-rose-500 translate-z-4 rotate-x-30" />
          </View>
        </View>
        <View className="perspective-normal">
          <View className={`${box} backface-hidden rotate-y-180`} />
        </View>
      </View>
    </View>
  );
}
