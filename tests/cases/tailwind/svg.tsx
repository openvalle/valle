export const composition = { width: 360, height: 160, fps: 1, duration: 1 };

// SVG with classes: fills and strokes of the palette, at an opacity, written, of the text color
// and none; stroke widths; opacity on shapes and groups; paint on a group its shapes inherit and
// shapes that put it back; and an <svg> sized, turned, shadowed and rounded by classes.
export default function Scene() {
  return (
    <View className="flex flex-wrap gap-2 p-2 text-rose-600">
      <svg width={72} height={48} className="bg-slate-100">
        <rect x={4} y={4} width={28} height={40} className="fill-sky-500 stroke-sky-900 stroke-2" />
        <circle cx={52} cy={24} r={16} className="fill-emerald-500/50 stroke-current stroke-[3]" />
      </svg>
      <svg width={72} height={48} className="bg-slate-100">
        <rect x={4} y={4} width={28} height={40} className="fill-current" />
        <circle cx={52} cy={24} r={16} className="fill-none stroke-[#7c3aed] stroke-1" />
      </svg>
      <svg width={72} height={48} className="bg-slate-100">
        <g className="fill-amber-400 opacity-75">
          <rect x={4} y={4} width={28} height={40} />
          <rect x={40} y={4} width={28} height={40} className="fill-blue-600 opacity-50" />
        </g>
      </svg>
      <svg width={72} height={48} className="bg-slate-100 stroke-teal-600 stroke-[4]">
        <path d="M6 42 L24 8 L42 42 L66 8" className="fill-none" />
      </svg>
      <svg viewBox="0 0 24 24" className="size-12 rotate-12 fill-indigo-500">
        <path d="M12 2 L22 22 H2 Z" />
      </svg>
      <svg
        viewBox="0 0 24 24"
        className="size-14 rounded-full bg-amber-100 p-2 shadow-md fill-amber-600"
      >
        <circle cx={12} cy={12} r={10} />
      </svg>
      <svg viewBox="0 0 24 24" className="size-12 fill-pink-500 drop-shadow-md">
        <rect x={3} y={3} width={18} height={18} rx={4} />
      </svg>
      <svg
        viewBox="0 0 24 24"
        className="w-20 h-12 border-2 border-slate-400 bg-white fill-lime-500"
      >
        <circle cx={12} cy={12} r={9} />
      </svg>
    </View>
  );
}
