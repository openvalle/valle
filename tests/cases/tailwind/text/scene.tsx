export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Text classes: font sizes of the scale with their line heights, and written ones with a line
// height after the slash; leading and tracking; weights, italic and the narrowest stretch; case,
// alignment, decoration lines with their style, thickness, offset and color; truncating,
// clamping, balancing and breaking; numeric variants; and the font stack.
export default function Scene() {
  return (
    <View
      className="flex flex-wrap items-start gap-x-3 gap-y-1 p-2 font-sans"
      style={{ background: "#eceff1" }}
    >
      <Text key="small" className="text-xs">
        Extra small
      </Text>
      <Text key="roomy" className="text-sm/6">
        Small, roomy
      </Text>
      <Text key="large" className="text-lg leading-none">
        Large
      </Text>
      <Text className="text-2xl font-black tracking-tight">Black</Text>
      <Text className="text-[13px]/[20px] font-thin italic">Thin italic</Text>
      <Text className="text-base tracking-widest uppercase">Caps</Text>
      <Text className="text-sm underline decoration-sky-500 decoration-2 underline-offset-4">
        Underlined
      </Text>
      <Text className="text-sm line-through decoration-dashed">Struck</Text>
      <Text className="text-sm font-stretch-ultra-condensed">Condensed text</Text>
      <Text key="truncate" className="w-24 truncate text-sm">
        A line too long to fit
      </Text>
      <Text key="clamp" className="line-clamp-2 w-28 text-xs leading-tight">
        Clamped after two lines of a longer paragraph of text
      </Text>
      <Text key="balance" className="w-32 text-center text-sm text-balance">
        Balanced lines of centered text here
      </Text>
      <Text key="break" className="w-20 text-xs break-all">
        Breakeverywhereword
      </Text>
      <Text className="text-sm tabular-nums ordinal">1st 111 999</Text>
      <Text className="w-24 text-right text-xs text-red-500/50">Right, half red</Text>
    </View>
  );
}
