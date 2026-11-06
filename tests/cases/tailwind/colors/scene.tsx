export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Color classes: backgrounds of the palette, black, white and transparent; at opacities a
// modifier gives, named and written, over a color; written colors; a written image with its size
// and position; text colors; and SVG fills and strokes, of the palette, `current`, written, at an
// opacity, with stroke widths of the scale and written.
export default function Scene() {
  const swatches = [
    "bg-red-500",
    "bg-orange-400",
    "bg-amber-300",
    "bg-lime-500",
    "bg-emerald-600",
    "bg-cyan-500",
    "bg-sky-700",
    "bg-indigo-500",
    "bg-fuchsia-500",
    "bg-rose-300",
    "bg-slate-800",
    "bg-zinc-200",
    "bg-stone-950",
    "bg-white",
    "bg-black",
    "bg-transparent",
  ];
  const faded = [
    "bg-black/50",
    "bg-sky-500/25",
    "bg-red-500/[0.7]",
    "bg-emerald-500/75",
    "bg-white/10",
    "bg-[#0af]/50",
  ];
  return (
    <View className="flex flex-wrap items-start gap-2 p-2 bg-neutral-100">
      <View className="flex flex-wrap w-64 gap-1">
        {swatches.map((c) => (
          <View key={c} className={`size-3 ${c}`} />
        ))}
      </View>
      <View className="flex gap-1 p-1 bg-yellow-300">
        {faded.map((c) => (
          <View key={c} className={`w-6 h-5 ${c}`} />
        ))}
      </View>
      <View className="flex gap-1">
        <View className="size-5 bg-[#7c4dff]" />
        <View className="size-5 bg-[rgb(10_200_120)]" />
        <View className="size-5 bg-[hsl(330_80%_60%)]" />
        <View className="size-5 bg-[color:tomato]" />
        <View className="size-5 bg-[oklch(70%_0.15_250)]" />
      </View>
      <View className="w-24 h-8 bg-[linear-gradient(90deg,#e53935,#1e88e5)] bg-[length:16px_100%]" />
      <View className="w-24 h-8 bg-[linear-gradient(#43a047,#fdd835)] bg-[length:40px_20px] bg-[center_top] bg-amber-100" />
      <View className="flex flex-col text-sm font-bold">
        <Text className="text-red-600">Red</Text>
        <Text className="text-sky-500/50">Sky half</Text>
        <Text className="text-[#8e24aa]">Purple</Text>
      </View>
      <svg width={180} height={50}>
        <circle cx={22} cy={25} r={18} className="fill-amber-400 stroke-indigo-600 stroke-2" />
        <circle cx={66} cy={25} r={18} className="fill-none stroke-rose-500 stroke-[3px]" />
        <g className="text-emerald-600">
          <rect
            x={92}
            y={7}
            width={36}
            height={36}
            className="fill-current stroke-black stroke-1"
          />
        </g>
        <rect
          x={140}
          y={7}
          width={36}
          height={36}
          className="fill-[#0af]/50 stroke-sky-900/50 stroke-[6]"
        />
      </svg>
    </View>
  );
}
